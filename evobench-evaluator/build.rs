fn main() {
    let debug_linear: bool;
    match std::env::var("DEBUG_LINEAR") {
        Ok(s) => {
            debug_linear = match s.as_str() {
                "1" | "t" | "y" => true,
                "0" | "f" | "n" => false,
                _ => panic!("invalid value for env var DEBUG_LINEAR"),
            };
        }
        Err(e) => match e {
            std::env::VarError::NotPresent => {
                debug_linear = false;
            }
            std::env::VarError::NotUnicode(_) => panic!("non-utf8 string in env var DEBUG_LINEAR"),
        },
    }
    if debug_linear {
        println!("cargo:rustc-cfg=feature=\"debug_linear\"");
    }
    println!("cargo::rerun-if-env-changed=DEBUG_LINEAR");
}
