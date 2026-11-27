fn main() {
    let cfg = slint_build::CompilerConfiguration::new().with_style("fluent".into());
    slint_build::compile_with_config("ui/app.slint", cfg).unwrap();
    // carimbo do build (America/Sao_Paulo); o script de publicação refaz o build a cada versão
    let stamp = std::process::Command::new("date")
        .env("TZ", "America/Sao_Paulo")
        .arg("+%d/%m/%Y %H:%M")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=RM_STAMP={stamp}");
}
