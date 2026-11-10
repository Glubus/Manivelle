use manivelle::{EngineError, host_fn};

struct Math;

impl Math {
    #[host_fn(name = "math.one")]
    fn one(&self) -> Result<f64, EngineError> {
        Ok(1.0)
    }
}

fn main() {}
