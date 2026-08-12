use std::{env, path::PathBuf};

fn main() {
    let mut args = env::args_os().skip(1);
    let Some(flag) = args.next() else {
        fail("expected --output <directory>");
    };
    let Some(path) = args.next() else {
        fail("expected --output <directory>");
    };
    if flag != "--output" || args.next().is_some() {
        fail("expected exactly --output <directory>");
    }
    let root = PathBuf::from(path);
    if let Err(error) = prepare_output_root(&root).and_then(|_| {
        p2_backend_common::fixture::generate_all(&root)
            .map(|_| ())
            .map_err(std::io::Error::other)
    }) {
        fail(&error.to_string());
    }
    println!("{{\"schema\":\"python-slm-backend-fixture-set-v1\",\"status\":\"PASS\"}}");
}

fn prepare_output_root(root: &std::path::Path) -> std::io::Result<()> {
    match std::fs::create_dir(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !root.is_dir() {
                return Err(error);
            }
            if std::fs::read_dir(root)?.next().is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "fixture output directory is not empty",
                ));
            }
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn fail(message: &str) -> ! {
    eprintln!(
        "{{\"code\":\"FIXTURE_GENERATION_FAILED\",\"message\":{}}}",
        serde_json::to_string(message).expect("message JSON")
    );
    std::process::exit(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "python-slm-p2-fixtures-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn accepts_a_precreated_empty_owned_directory() {
        let root = temporary_path("empty");
        std::fs::create_dir(&root).unwrap();
        prepare_output_root(&root).unwrap();
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn rejects_a_nonempty_directory() {
        let root = temporary_path("nonempty");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("foreign"), b"x").unwrap();
        assert_eq!(
            prepare_output_root(&root).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
