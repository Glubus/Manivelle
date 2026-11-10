use manivelle::{EngineError, host_fn};

#[host_fn(name = "math.identity")]
fn identity<T>(value: f64) -> Result<f64, EngineError> {
    Ok(value)
}

fn main() {}
