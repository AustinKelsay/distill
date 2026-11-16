//! The MCP tools, one module each, so tickets built in parallel don't edit the same file. A new
//! tool adds its module here and its entry to [`all`].

use distill_core::Home;
use serde_json::Value;

/// One MCP tool.
pub trait Tool {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    /// JSON Schema for the tool's arguments.
    fn input_schema(&self) -> Value;
    /// Run the tool. An `Err` is returned to the client as a tool error.
    fn call(&self, home: &Home, args: &Value) -> Result<Value, String>;
}

/// Every tool the server offers, in the order `tools/list` shows them.
pub fn all() -> Vec<Box<dyn Tool>> {
    Vec::new()
}
