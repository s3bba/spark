fn main() {
    println!("cargo::rerun-if-env-changed=SPARK_EMBEDDED_FONT_FILE");
    println!("cargo::rustc-check-cfg=cfg(spark_embedded_font)");

    if let Ok(path) = std::env::var("SPARK_EMBEDDED_FONT_FILE") {
        println!("cargo::rustc-cfg=spark_embedded_font");
        println!("cargo::rustc-env=SPARK_EMBEDDED_FONT_FILE={path}");
    }
}
