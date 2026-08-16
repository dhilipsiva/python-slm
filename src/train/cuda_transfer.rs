//! CUDA Driver API transfer adapter with page-locked staging on Windows and Linux hosts.
//!
//! Windows loads `nvcuda.dll` from System32 only; Linux loads `libcuda.so.1` with
//! `RTLD_NOW | RTLD_LOCAL`. Every driver call, ticket ownership rule, and cleanup
//! order is identical across both hosts.

use super::{AsyncBatchTransfer, LoadedSpan};
use crate::error::{ProductError, Result};
use std::ffi::{CStr, c_void};
use std::ptr;
use std::sync::Arc;

type CuResult = i32;
type CuDevice = i32;
type CuContext = *mut c_void;
type CuStream = *mut c_void;
type CuDevicePtr = u64;

type CuInit = unsafe extern "C" fn(u32) -> CuResult;
type CuDeviceGet = unsafe extern "C" fn(*mut CuDevice, i32) -> CuResult;
type CuDevicePrimaryCtxRetain = unsafe extern "C" fn(*mut CuContext, CuDevice) -> CuResult;
type CuDevicePrimaryCtxRelease = unsafe extern "C" fn(CuDevice) -> CuResult;
type CuCtxPushCurrent = unsafe extern "C" fn(CuContext) -> CuResult;
type CuCtxPopCurrent = unsafe extern "C" fn(*mut CuContext) -> CuResult;
type CuMemAllocHost = unsafe extern "C" fn(*mut *mut c_void, usize) -> CuResult;
type CuMemFreeHost = unsafe extern "C" fn(*mut c_void) -> CuResult;
type CuMemAlloc = unsafe extern "C" fn(*mut CuDevicePtr, usize) -> CuResult;
type CuMemFree = unsafe extern "C" fn(CuDevicePtr) -> CuResult;
type CuMemcpyHtoDAsync =
    unsafe extern "C" fn(CuDevicePtr, *const c_void, usize, CuStream) -> CuResult;
type CuStreamCreate = unsafe extern "C" fn(*mut CuStream, u32) -> CuResult;
type CuStreamSynchronize = unsafe extern "C" fn(CuStream) -> CuResult;
type CuStreamDestroy = unsafe extern "C" fn(CuStream) -> CuResult;

#[cfg(windows)]
struct DriverLibrary(windows_sys::Win32::Foundation::HMODULE);

#[cfg(target_os = "linux")]
struct DriverLibrary(*mut c_void);

fn driver_load_failed() -> ProductError {
    ProductError::environment(
        "P11_CUDA_DRIVER_LOAD_FAILED",
        "the NVIDIA CUDA driver library could not be loaded from its native system location",
    )
}

impl DriverLibrary {
    #[cfg(windows)]
    fn open() -> Result<Self> {
        use windows_sys::Win32::System::LibraryLoader::{
            LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
        };
        let name = "nvcuda.dll".encode_utf16().chain([0]).collect::<Vec<_>>();
        // SAFETY: the UTF-16 name is NUL-terminated and SYSTEM32-only search prevents DLL planting.
        let module =
            unsafe { LoadLibraryExW(name.as_ptr(), ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() {
            return Err(driver_load_failed());
        }
        Ok(Self(module))
    }

    #[cfg(target_os = "linux")]
    fn open() -> Result<Self> {
        // SAFETY: the soname literal is NUL-terminated and the returned handle is uniquely owned.
        let handle =
            unsafe { libc::dlopen(c"libcuda.so.1".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err(driver_load_failed());
        }
        Ok(Self(handle))
    }

    #[cfg(windows)]
    fn raw_symbol(&self, name: &CStr) -> Option<*mut c_void> {
        use windows_sys::Win32::System::LibraryLoader::GetProcAddress;
        // SAFETY: the module is live for the wrapper lifetime and the name is NUL-terminated.
        unsafe { GetProcAddress(self.0, name.as_ptr().cast()) }.map(|symbol| symbol as *mut c_void)
    }

    #[cfg(target_os = "linux")]
    fn raw_symbol(&self, name: &CStr) -> Option<*mut c_void> {
        // SAFETY: the handle is live for the wrapper lifetime and the name is NUL-terminated.
        let symbol = unsafe { libc::dlsym(self.0, name.as_ptr()) };
        (!symbol.is_null()).then_some(symbol)
    }
}

impl Drop for DriverLibrary {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: the module handle is uniquely owned by this wrapper.
        unsafe {
            windows_sys::Win32::Foundation::FreeLibrary(self.0)
        };
        #[cfg(target_os = "linux")]
        // SAFETY: the dlopen handle is uniquely owned by this wrapper.
        unsafe {
            libc::dlclose(self.0)
        };
    }
}

struct CudaDriver {
    #[allow(
        dead_code,
        reason = "owns the loaded driver library for the function lifetimes"
    )]
    library: DriverLibrary,
    init: CuInit,
    device_get: CuDeviceGet,
    primary_retain: CuDevicePrimaryCtxRetain,
    primary_release: CuDevicePrimaryCtxRelease,
    context_push: CuCtxPushCurrent,
    context_pop: CuCtxPopCurrent,
    host_alloc: CuMemAllocHost,
    host_free: CuMemFreeHost,
    device_alloc: CuMemAlloc,
    device_free: CuMemFree,
    copy_htod_async: CuMemcpyHtoDAsync,
    stream_create: CuStreamCreate,
    stream_synchronize: CuStreamSynchronize,
    stream_destroy: CuStreamDestroy,
}

// SAFETY: the module remains loaded for the struct lifetime and CUDA Driver API entry points are
// process-global, thread-safe functions. CUDA contexts are explicitly pushed per operation.
unsafe impl Send for CudaDriver {}
// SAFETY: see the Send justification; all mutable CUDA state is owned by individual tickets.
unsafe impl Sync for CudaDriver {}

impl CudaDriver {
    fn load() -> Result<Arc<Self>> {
        let library = DriverLibrary::open()?;
        let driver = Self {
            init: load_symbol(&library, c"cuInit")?,
            device_get: load_symbol(&library, c"cuDeviceGet")?,
            primary_retain: load_symbol(&library, c"cuDevicePrimaryCtxRetain")?,
            primary_release: load_symbol_any(
                &library,
                &[
                    c"cuDevicePrimaryCtxRelease_v2",
                    c"cuDevicePrimaryCtxRelease",
                ],
            )?,
            context_push: load_symbol_any(
                &library,
                &[c"cuCtxPushCurrent_v2", c"cuCtxPushCurrent"],
            )?,
            context_pop: load_symbol_any(&library, &[c"cuCtxPopCurrent_v2", c"cuCtxPopCurrent"])?,
            host_alloc: load_symbol_any(&library, &[c"cuMemAllocHost_v2", c"cuMemAllocHost"])?,
            host_free: load_symbol(&library, c"cuMemFreeHost")?,
            device_alloc: load_symbol_any(&library, &[c"cuMemAlloc_v2", c"cuMemAlloc"])?,
            device_free: load_symbol_any(&library, &[c"cuMemFree_v2", c"cuMemFree"])?,
            copy_htod_async: load_symbol_any(
                &library,
                &[c"cuMemcpyHtoDAsync_v2", c"cuMemcpyHtoDAsync"],
            )?,
            stream_create: load_symbol(&library, c"cuStreamCreate")?,
            stream_synchronize: load_symbol(&library, c"cuStreamSynchronize")?,
            stream_destroy: load_symbol_any(
                &library,
                &[c"cuStreamDestroy_v2", c"cuStreamDestroy"],
            )?,
            library,
        };
        Ok(Arc::new(driver))
    }
}

fn load_symbol<T: Copy>(library: &DriverLibrary, name: &CStr) -> Result<T> {
    load_symbol_any(library, &[name])
}

fn load_symbol_any<T: Copy>(library: &DriverLibrary, names: &[&CStr]) -> Result<T> {
    for name in names {
        if let Some(symbol) = library.raw_symbol(name) {
            if size_of::<T>() != size_of_val(&symbol) {
                break;
            }
            // SAFETY: every caller supplies the exact CUDA signature for the requested symbol.
            return Ok(unsafe { std::mem::transmute_copy(&symbol) });
        }
    }
    Err(ProductError::environment(
        "P11_CUDA_DRIVER_SYMBOL_MISSING",
        "the CUDA driver does not expose a required transfer entry point",
    ))
}

fn check(code: CuResult, operation: &'static str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(ProductError::environment(
            "P11_CUDA_DRIVER_CALL_FAILED",
            format!("CUDA driver operation {operation} failed with code {code}"),
        ))
    }
}

pub struct CudaPinnedTransfer {
    driver: Arc<CudaDriver>,
    device: CuDevice,
}

impl CudaPinnedTransfer {
    pub fn new(device_ordinal: u32) -> Result<Self> {
        let ordinal = i32::try_from(device_ordinal).map_err(|_| {
            ProductError::usage(
                "P11_CUDA_DEVICE_INVALID",
                "the CUDA device ordinal is invalid",
            )
        })?;
        let driver = CudaDriver::load()?;
        // SAFETY: fixed CUDA initialization call with no user pointers.
        check(unsafe { (driver.init)(0) }, "cuInit")?;
        let mut device = 0;
        // SAFETY: device points to initialized writable storage for this call.
        check(
            unsafe { (driver.device_get)(&mut device, ordinal) },
            "cuDeviceGet",
        )?;
        Ok(Self { driver, device })
    }
}

pub struct CudaTransferTicket {
    driver: Arc<CudaDriver>,
    device: CuDevice,
    context: CuContext,
    host: *mut c_void,
    device_ptr: CuDevicePtr,
    stream: CuStream,
    split: crate::storage::CorpusSplit,
    sequence: u64,
    first_id: u64,
    valid_targets: u64,
    bytes: usize,
}

// SAFETY: the retained primary context can be pushed on the receiving thread; all allocations are
// uniquely owned by the ticket and CUDA synchronizes before their release.
unsafe impl Send for CudaTransferTicket {}

pub struct CudaDeviceBatch {
    driver: Arc<CudaDriver>,
    device: CuDevice,
    context: CuContext,
    device_ptr: CuDevicePtr,
    pub split: crate::storage::CorpusSplit,
    pub sequence: u64,
    pub first_id: u64,
    pub valid_targets: u64,
    pub bytes: usize,
}

// SAFETY: the device allocation is uniquely owned and its primary context can be pushed on any
// receiving thread before release.
unsafe impl Send for CudaDeviceBatch {}

impl CudaDeviceBatch {
    #[allow(dead_code, reason = "P12 consumes the opaque device binding")]
    pub(crate) fn device_binding(&self) -> u64 {
        self.device_ptr
    }
}

impl AsyncBatchTransfer for CudaPinnedTransfer {
    type Ticket = CudaTransferTicket;
    type DeviceBatch = CudaDeviceBatch;

    fn submit(&mut self, span: LoadedSpan) -> Result<Self::Ticket> {
        let bytes = span.bytes();
        let mut ticket = CudaTransferTicket {
            driver: self.driver.clone(),
            device: self.device,
            context: ptr::null_mut(),
            host: ptr::null_mut(),
            device_ptr: 0,
            stream: ptr::null_mut(),
            split: span.split,
            sequence: span.sequence,
            first_id: span.first_id,
            valid_targets: span.valid_targets,
            bytes,
        };
        // SAFETY: all out-pointers refer to fields owned by the ticket; cleanup runs on any error.
        let result = unsafe { ticket.allocate_and_submit(span.token_ids()) };
        if let Err(error) = result {
            ticket.cleanup(true);
            return Err(error);
        }
        Ok(ticket)
    }

    fn wait(&mut self, mut ticket: Self::Ticket) -> Result<Self::DeviceBatch> {
        ticket.synchronize()?;
        ticket.release_host_and_stream()?;
        let batch = CudaDeviceBatch {
            driver: ticket.driver.clone(),
            device: ticket.device,
            context: ticket.context,
            device_ptr: ticket.device_ptr,
            split: ticket.split,
            sequence: ticket.sequence,
            first_id: ticket.first_id,
            valid_targets: ticket.valid_targets,
            bytes: ticket.bytes,
        };
        ticket.context = ptr::null_mut();
        ticket.device_ptr = 0;
        Ok(batch)
    }

    fn cancel(&mut self, mut ticket: Self::Ticket) {
        ticket.cleanup(true);
    }
}

impl CudaTransferTicket {
    unsafe fn allocate_and_submit(&mut self, tokens: &[u16]) -> Result<()> {
        check(
            // SAFETY: context points to writable ticket storage and device is validated.
            unsafe { (self.driver.primary_retain)(&mut self.context, self.device) },
            "cuDevicePrimaryCtxRetain",
        )?;
        self.push_context()?;
        let operation = (|| {
            check(
                // SAFETY: host points to writable ticket storage and bytes is nonzero.
                unsafe { (self.driver.host_alloc)(&mut self.host, self.bytes) },
                "cuMemAllocHost",
            )?;
            // SAFETY: host owns at least bytes and tokens contains exactly bytes readable bytes.
            unsafe {
                ptr::copy_nonoverlapping(tokens.as_ptr().cast::<u8>(), self.host.cast(), self.bytes)
            };
            check(
                // SAFETY: device_ptr points to writable ticket storage.
                unsafe { (self.driver.device_alloc)(&mut self.device_ptr, self.bytes) },
                "cuMemAlloc",
            )?;
            check(
                // SAFETY: stream points to writable ticket storage in the current context.
                unsafe { (self.driver.stream_create)(&mut self.stream, 1) },
                "cuStreamCreate",
            )?;
            check(
                // SAFETY: host and device allocations cover bytes and stream is current-context owned.
                unsafe {
                    (self.driver.copy_htod_async)(
                        self.device_ptr,
                        self.host,
                        self.bytes,
                        self.stream,
                    )
                },
                "cuMemcpyHtoDAsync",
            )
        })();
        let pop = self.pop_context();
        operation.and(pop)
    }

    fn push_context(&self) -> Result<()> {
        // SAFETY: context is a retained primary context.
        check(
            unsafe { (self.driver.context_push)(self.context) },
            "cuCtxPushCurrent",
        )
    }

    fn pop_context(&self) -> Result<()> {
        let mut popped = ptr::null_mut();
        // SAFETY: a matching retained context was pushed on this thread.
        check(
            unsafe { (self.driver.context_pop)(&mut popped) },
            "cuCtxPopCurrent",
        )?;
        if popped != self.context {
            return Err(ProductError::internal(
                "P11_CUDA_CONTEXT_MISMATCH",
                "CUDA returned a different context at the transfer boundary",
            ));
        }
        Ok(())
    }

    fn synchronize(&self) -> Result<()> {
        self.push_context()?;
        let operation = check(
            // SAFETY: stream is owned by this ticket in the pushed context.
            unsafe { (self.driver.stream_synchronize)(self.stream) },
            "cuStreamSynchronize",
        );
        let pop = self.pop_context();
        operation.and(pop)
    }

    fn release_host_and_stream(&mut self) -> Result<()> {
        self.push_context()?;
        let mut first_error = None;
        if !self.stream.is_null() {
            // SAFETY: stream is owned and synchronized.
            let result = unsafe { (self.driver.stream_destroy)(self.stream) };
            match check(result, "cuStreamDestroy") {
                Ok(()) => self.stream = ptr::null_mut(),
                Err(error) => first_error = Some(error),
            }
        }
        if !self.host.is_null() {
            // SAFETY: host was allocated by cuMemAllocHost and transfer is synchronized.
            let result = unsafe { (self.driver.host_free)(self.host) };
            match check(result, "cuMemFreeHost") {
                Ok(()) => self.host = ptr::null_mut(),
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        if let Err(error) = self.pop_context()
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        first_error.map_or(Ok(()), Err)
    }

    fn cleanup(&mut self, synchronize: bool) {
        if self.context.is_null() {
            return;
        }
        if synchronize && !self.stream.is_null() {
            let _ = self.synchronize();
        }
        let _ = self.push_context();
        if !self.stream.is_null() {
            // SAFETY: best-effort release of a stream uniquely owned by this ticket.
            unsafe { (self.driver.stream_destroy)(self.stream) };
            self.stream = ptr::null_mut();
        }
        if self.device_ptr != 0 {
            // SAFETY: best-effort release of an allocation uniquely owned by this ticket.
            unsafe { (self.driver.device_free)(self.device_ptr) };
            self.device_ptr = 0;
        }
        if !self.host.is_null() {
            // SAFETY: best-effort release of an allocation uniquely owned by this ticket.
            unsafe { (self.driver.host_free)(self.host) };
            self.host = ptr::null_mut();
        }
        let _ = self.pop_context();
        // SAFETY: this ticket owns exactly one retained primary-context reference.
        unsafe { (self.driver.primary_release)(self.device) };
        self.context = ptr::null_mut();
    }
}

impl Drop for CudaTransferTicket {
    fn drop(&mut self) {
        self.cleanup(true);
    }
}

impl Drop for CudaDeviceBatch {
    fn drop(&mut self) {
        if self.context.is_null() {
            return;
        }
        // SAFETY: this batch uniquely owns the device allocation and retained context reference.
        unsafe {
            if (self.driver.context_push)(self.context) == 0 {
                (self.driver.device_free)(self.device_ptr);
                let mut popped = ptr::null_mut();
                (self.driver.context_pop)(&mut popped);
            }
            (self.driver.primary_release)(self.device);
        }
        self.context = ptr::null_mut();
        self.device_ptr = 0;
    }
}
