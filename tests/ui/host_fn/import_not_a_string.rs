use manivelle::{EngineError, host_fn, host_mod};

#[host_mod(namespace = "math", import = app)]
mod math {
    use super::*;

    #[host_fn]
    pub fn add(input: f64) -> Result<f64, EngineError> {
        Ok(input)
    }
}

fn main() {}
