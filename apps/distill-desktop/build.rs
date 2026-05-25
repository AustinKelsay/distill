fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-dark".to_string());
    slint_build::compile_with_config("ui/shell.slint", config).expect("failed to compile Slint UI");
}
