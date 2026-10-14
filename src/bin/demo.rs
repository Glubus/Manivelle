use manivelle::{Engine, VmOptions};
use serde_json::{Value, json};

const DEMO_SCRIPT: &str = include_str!("../../assets/demo_math.ts");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new(&VmOptions::default())?;
    engine.load_script("math", DEMO_SCRIPT)?;
    let result: Value = engine.call("math", "sum", (json!({ "left": 20, "right": 22 }),))?;

    println!("result: {result}");
    println!("memory: {:?}", engine.memory_stats());
    Ok(())
}
