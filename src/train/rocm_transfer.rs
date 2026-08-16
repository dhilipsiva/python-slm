//! Linux HIP runtime transfer adapter with page-locked staging.
//!
//! The adapter loads AMD's HIP runtime dynamically, allocates true page-locked
//! host staging with `hipHostMalloc`, and submits a nonblocking
//! `hipMemcpyHtoDAsync` on an owned stream. Ticket ownership, source-order
//! retirement, synchronization-before-release, and reverse-order cleanup follow
//! the exact P11 discrete-staging semantics.

use super::{AsyncBatchTransfer, LoadedSpan};
use crate::error::{ProductError, Result};
use std::ffi::{CStr, c_void};
use std::ptr;
use std::sync::Arc;

type HipResult = i32;
type HipStream = *mut c_void;
type HipDevicePtr = *mut c_void;

const HIP_STREAM_NON_BLOCKING: u32 = 1;

type HipInit = unsafe extern "C" fn(u32) -> HipResult;
type HipSetDevice = unsafe extern "C" fn(i32) -> HipResult;
type HipHostMalloc = unsafe extern "C" fn(*mut *mut c_void, usize, u32) -> HipResult;
type HipHostFree = unsafe extern "C" fn(*mut c_void) -> HipResult;
type HipMalloc = unsafe extern "C" fn(*mut HipDevicePtr, usize) -> HipResult;
type HipFree = unsafe extern "C" fn(HipDevicePtr) -> HipResult;
type HipMemcpyHtoDAsync =
    unsafe extern "C" fn(HipDevicePtr, *const c_void, usize, HipStream) -> HipResult;
type HipStreamCreateWithFlags = unsafe extern "C" fn(*mut HipStream, u32) -> HipResult;
type HipStreamSynchronize = unsafe extern "C" fn(HipStream) -> HipResult;
type HipStreamDestroy = unsafe extern "C" fn(HipStream) -> HipResult;

const HIP_LIBRARY_CANDIDATES: [&CStr; 4] = [
    c"libamdhip64.so",
    c"libamdhip64.so.7",
    c"libamdhip64.so.6",
    c"libamdhip64.so.5",
];

struct RuntimeLibrary(*mut c_void);

impl RuntimeLibrary {
    fn open() -> Result<Self> {
        for name in HIP_LIBRARY_CANDIDATES {
            // SAFETY: the soname literal is NUL-terminated and the handle is uniquely owned.
            let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            if !handle.is_null() {
                return Ok(Self(handle));
            }
        }
        Err(ProductError::environment(
            "P18_HIP_RUNTIME_LOAD_FAILED",
            "the AMD HIP runtime library could not be loaded from its native system location",
        ))
    }

    fn raw_symbol(&self, name: &CStr) -> Option<*mut c_void> {
        // SAFETY: the handle is live for the wrapper lifetime and the name is NUL-terminated.
        let symbol = unsafe { libc::dlsym(self.0, name.as_ptr()) };
        (!symbol.is_null()).then_some(symbol)
    }
}

impl Drop for RuntimeLibrary {
    fn drop(&mut self) {
        // SAFETY: the dlopen handle is uniquely owned by this wrapper.
        unsafe { libc::dlclose(self.0) };
    }
}

fn load_symbol<T: Copy>(library: &RuntimeLibrary, name: &CStr) -> Result<T> {
    if let Some(symbol) = library.raw_symbol(name)
        && size_of::<T>() == size_of_val(&symbol)
    {
        // SAFETY: every caller supplies the exact HIP signature for the requested symbol.
        return Ok(unsafe { std::mem::transmute_copy(&symbol) });
    }
    Err(ProductError::environment(
        "P18_HIP_RUNTIME_SYMBOL_MISSING",
        "the HIP runtime does not expose a required transfer entry point",
    ))
}

struct HipRuntime {
    #[allow(
        dead_code,
        reason = "owns the loaded runtime library for the function lifetimes"
    )]
    library: RuntimeLibrary,
    init: HipInit,
    set_device: HipSetDevice,
    host_alloc: HipHostMalloc,
    host_free: HipHostFree,
    device_alloc: HipMalloc,
    device_free: HipFree,
    copy_htod_async: HipMemcpyHtoDAsync,
    stream_create: HipStreamCreateWithFlags,
    stream_synchronize: HipStreamSynchronize,
    stream_destroy: HipStreamDestroy,
}

// SAFETY: the library remains loaded for the struct lifetime and HIP runtime entry points are
// process-global, thread-safe functions; the device is re-bound per operation with hipSetDevice.
unsafe impl Send for HipRuntime {}
// SAFETY: see the Send justification; all mutable HIP state is owned by individual tickets.
unsafe impl Sync for HipRuntime {}

impl HipRuntime {
    fn load() -> Result<Arc<Self>> {
        let library = RuntimeLibrary::open()?;
        let runtime = Self {
            init: load_symbol(&library, c"hipInit")?,
            set_device: load_symbol(&library, c"hipSetDevice")?,
            host_alloc: load_symbol(&library, c"hipHostMalloc")?,
            host_free: load_symbol(&library, c"hipHostFree")?,
            device_alloc: load_symbol(&library, c"hipMalloc")?,
            device_free: load_symbol(&library, c"hipFree")?,
            copy_htod_async: load_symbol(&library, c"hipMemcpyHtoDAsync")?,
            stream_create: load_symbol(&library, c"hipStreamCreateWithFlags")?,
            stream_synchronize: load_symbol(&library, c"hipStreamSynchronize")?,
            stream_destroy: load_symbol(&library, c"hipStreamDestroy")?,
            library,
        };
        Ok(Arc::new(runtime))
    }
}

fn check(code: HipResult, operation: &'static str) -> Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(ProductError::environment(
            "P18_HIP_RUNTIME_CALL_FAILED",
            format!("HIP runtime operation {operation} failed with code {code}"),
        ))
    }
}

pub struct RocmPinnedTransfer {
    runtime: Arc<HipRuntime>,
    device: i32,
}

impl RocmPinnedTransfer {
    pub fn new(device_ordinal: u32) -> Result<Self> {
        let device = i32::try_from(device_ordinal).map_err(|_| {
            ProductError::usage(
                "P18_HIP_DEVICE_INVALID",
                "the HIP device ordinal is invalid",
            )
        })?;
        let runtime = HipRuntime::load()?;
        // SAFETY: fixed HIP initialization call with no user pointers.
        check(unsafe { (runtime.init)(0) }, "hipInit")?;
        // SAFETY: hipSetDevice validates the ordinal and binds it to the calling thread.
        check(unsafe { (runtime.set_device)(device) }, "hipSetDevice")?;
        Ok(Self { runtime, device })
    }
}

pub struct RocmTransferTicket {
    runtime: Arc<HipRuntime>,
    device: i32,
    host: *mut c_void,
    device_ptr: HipDevicePtr,
    stream: HipStream,
    split: crate::storage::CorpusSplit,
    sequence: u64,
    first_id: u64,
    valid_targets: u64,
    bytes: usize,
}

// SAFETY: the device is re-bound with hipSetDevice on the receiving thread before every
// operation; all allocations are uniquely owned by the ticket and HIP synchronizes before
// their release.
unsafe impl Send for RocmTransferTicket {}

pub struct RocmDeviceBatch {
    runtime: Arc<HipRuntime>,
    device: i32,
    device_ptr: HipDevicePtr,
    pub split: crate::storage::CorpusSplit,
    pub sequence: u64,
    pub first_id: u64,
    pub valid_targets: u64,
    pub bytes: usize,
}

// SAFETY: the device allocation is uniquely owned and the device is re-bound on any receiving
// thread before release.
unsafe impl Send for RocmDeviceBatch {}

impl AsyncBatchTransfer for RocmPinnedTransfer {
    type Ticket = RocmTransferTicket;
    type DeviceBatch = RocmDeviceBatch;

    fn submit(&mut self, span: LoadedSpan) -> Result<Self::Ticket> {
        let bytes = span.bytes();
        let mut ticket = RocmTransferTicket {
            runtime: self.runtime.clone(),
            device: self.device,
            host: ptr::null_mut(),
            device_ptr: ptr::null_mut(),
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
        let batch = RocmDeviceBatch {
            runtime: ticket.runtime.clone(),
            device: ticket.device,
            device_ptr: ticket.device_ptr,
            split: ticket.split,
            sequence: ticket.sequence,
            first_id: ticket.first_id,
            valid_targets: ticket.valid_targets,
            bytes: ticket.bytes,
        };
        ticket.device_ptr = ptr::null_mut();
        Ok(batch)
    }

    fn cancel(&mut self, mut ticket: Self::Ticket) {
        ticket.cleanup(true);
    }
}

impl RocmTransferTicket {
    fn bind_device(&self) -> Result<()> {
        // SAFETY: the ordinal was validated at transfer construction.
        check(
            unsafe { (self.runtime.set_device)(self.device) },
            "hipSetDevice",
        )
    }

    unsafe fn allocate_and_submit(&mut self, tokens: &[u16]) -> Result<()> {
        self.bind_device()?;
        check(
            // SAFETY: host points to writable ticket storage and bytes is nonzero.
            unsafe { (self.runtime.host_alloc)(&mut self.host, self.bytes, 0) },
            "hipHostMalloc",
        )?;
        // SAFETY: host owns at least bytes and tokens contains exactly bytes readable bytes.
        unsafe {
            ptr::copy_nonoverlapping(tokens.as_ptr().cast::<u8>(), self.host.cast(), self.bytes)
        };
        check(
            // SAFETY: device_ptr points to writable ticket storage.
            unsafe { (self.runtime.device_alloc)(&mut self.device_ptr, self.bytes) },
            "hipMalloc",
        )?;
        check(
            // SAFETY: stream points to writable ticket storage on the bound device.
            unsafe { (self.runtime.stream_create)(&mut self.stream, HIP_STREAM_NON_BLOCKING) },
            "hipStreamCreateWithFlags",
        )?;
        check(
            // SAFETY: host and device allocations cover bytes and stream is ticket-owned.
            unsafe {
                (self.runtime.copy_htod_async)(self.device_ptr, self.host, self.bytes, self.stream)
            },
            "hipMemcpyHtoDAsync",
        )
    }

    fn synchronize(&self) -> Result<()> {
        self.bind_device()?;
        check(
            // SAFETY: stream is owned by this ticket on the bound device.
            unsafe { (self.runtime.stream_synchronize)(self.stream) },
            "hipStreamSynchronize",
        )
    }

    fn release_host_and_stream(&mut self) -> Result<()> {
        self.bind_device()?;
        let mut first_error = None;
        if !self.stream.is_null() {
            // SAFETY: stream is owned and synchronized.
            let result = unsafe { (self.runtime.stream_destroy)(self.stream) };
            match check(result, "hipStreamDestroy") {
                Ok(()) => self.stream = ptr::null_mut(),
                Err(error) => first_error = Some(error),
            }
        }
        if !self.host.is_null() {
            // SAFETY: host was allocated by hipHostMalloc and the transfer is synchronized.
            let result = unsafe { (self.runtime.host_free)(self.host) };
            match check(result, "hipHostFree") {
                Ok(()) => self.host = ptr::null_mut(),
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn cleanup(&mut self, synchronize: bool) {
        if self.host.is_null() && self.device_ptr.is_null() && self.stream.is_null() {
            return;
        }
        if self.bind_device().is_err() {
            return;
        }
        if synchronize && !self.stream.is_null() {
            let _ = self.synchronize();
        }
        if !self.stream.is_null() {
            // SAFETY: best-effort release of a stream uniquely owned by this ticket.
            unsafe { (self.runtime.stream_destroy)(self.stream) };
            self.stream = ptr::null_mut();
        }
        if !self.device_ptr.is_null() {
            // SAFETY: best-effort release of an allocation uniquely owned by this ticket.
            unsafe { (self.runtime.device_free)(self.device_ptr) };
            self.device_ptr = ptr::null_mut();
        }
        if !self.host.is_null() {
            // SAFETY: best-effort release of an allocation uniquely owned by this ticket.
            unsafe { (self.runtime.host_free)(self.host) };
            self.host = ptr::null_mut();
        }
    }
}

impl Drop for RocmTransferTicket {
    fn drop(&mut self) {
        self.cleanup(true);
    }
}

impl Drop for RocmDeviceBatch {
    fn drop(&mut self) {
        if self.device_ptr.is_null() {
            return;
        }
        // SAFETY: this batch uniquely owns the device allocation; the device is re-bound first.
        unsafe {
            if (self.runtime.set_device)(self.device) == 0 {
                (self.runtime.device_free)(self.device_ptr);
            }
        }
        self.device_ptr = ptr::null_mut();
    }
}
