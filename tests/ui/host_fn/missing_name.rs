use manivelle::{EngineError, host_fn};

#[host_fn]
fn ping() -> Result<(), EngineError> {
    Ok(())
}

fn main() {}
