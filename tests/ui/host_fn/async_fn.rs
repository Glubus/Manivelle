use manivelle::{EngineError, host_fn};

#[host_fn(name = "math.one")]
async fn one() -> Result<f64, EngineError> {
    Ok(1.0)
}

fn main() {}
