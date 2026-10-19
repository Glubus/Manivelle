use manivelle::{host_fn, host_mod};

#[host_mod(namespace = "math")]
mod math {
    #[host_fn(no_namespace)]
    pub fn log() -> Result<(), manivelle::EngineError> {
        Ok(())
    }
}

fn main() {}
