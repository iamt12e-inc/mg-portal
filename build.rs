use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=MUGONAT_BUILD_NUMBER");

    let build_number = env::var("MUGONAT_BUILD_NUMBER").unwrap_or_else(|_| "local".to_owned());
    println!("cargo:rustc-env=MUGONAT_BUILD_NUMBER={build_number}");
}
