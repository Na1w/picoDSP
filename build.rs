use std::fs;

fn main() {
    println!("cargo:rustc-link-arg-bins=--nmagic");
    println!("cargo:rustc-link-arg-bins=-Tlink.x");
    println!("cargo:rustc-link-arg-bins=-Tdefmt.x");

    // --- RAM & Flash Calculation ---
    println!("cargo:rerun-if-changed=memory.x");

    let memory_x_content = fs::read_to_string("memory.x").unwrap();
    let mut total_ram_kb = 0;
    let mut total_flash_kb = 0;

    for line in memory_x_content.lines() {
        let line = line.trim();

        // RAM Calculation
        if (line.starts_with("RAM") || line.starts_with("SRAM")) && line.contains("LENGTH =") {
            if let Some(len_part) = line.split("LENGTH =").nth(1) {
                let len_str = len_part
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('K');
                if let Ok(kb) = len_str.parse::<u32>() {
                    total_ram_kb += kb;
                }
            }
        }

        // Flash Calculation
        if line.starts_with("FLASH") && line.contains("LENGTH =") {
            if let Some(len_part) = line.split("LENGTH =").nth(1) {
                let len_str = len_part
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('K');
                if let Ok(kb) = len_str.parse::<u32>() {
                    total_flash_kb += kb;
                }
            }
        }
    }

    println!("cargo:rustc-env=TOTAL_RAM_KB={}", total_ram_kb);
    println!("cargo:rustc-env=TOTAL_FLASH_KB={}", total_flash_kb);

    // --- Get infinitedsp-core version ---
    let version =
        get_dependency_version("infinitedsp-core").unwrap_or_else(|| "Unknown".to_string());
    println!("cargo:rustc-env=INFINITEDSP_CORE_VERSION={}", version);
}

fn get_dependency_version(pkg_name: &str) -> Option<String> {
    let lock_content = fs::read_to_string("Cargo.lock").ok()?;
    let mut in_package = false;
    let mut current_name = String::new();

    for line in lock_content.lines() {
        if line.trim() == "[[package]]" {
            in_package = true;
            current_name.clear();
            continue;
        }

        if in_package {
            let parts: Vec<&str> = line.split('=').map(|s| s.trim()).collect();
            if parts.len() == 2 {
                if parts[0] == "name" {
                    current_name = parts[1].trim_matches('"').to_string();
                } else if parts[0] == "version"
                    && current_name == pkg_name {
                        return Some(parts[1].trim_matches('"').to_string());
                    }
            }
        }
    }
    None
}
