//! Engine options: the ones that would turn a protection off are refused.

mod support;

use manivelle::{Engine, EngineError, EngineOptions};

#[test]
fn options_that_would_turn_a_protection_off_are_refused() {
    for options in [
        EngineOptions {
            max_stack_size_bytes: 0,
            ..EngineOptions::default()
        },
        EngineOptions {
            memory_limit_bytes: 0,
            ..EngineOptions::default()
        },
    ] {
        let result = Engine::new(&options);

        assert!(
            matches!(&result, Err(EngineError::InvalidOptions { details }) if details.contains("is 0")),
            "{:?}",
            result.err()
        );
    }
}
