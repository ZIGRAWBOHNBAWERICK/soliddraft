//! Embeds the source revision for browser feedback, including builds without Git metadata.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SOLIDDRAFT_COMMIT");
    let mut refs = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    if let Ok(output) = Command::new("git").args(["symbolic-ref", "-q", "HEAD"]).output()
        && output.status.success()
    {
        refs.push(String::from_utf8_lossy(&output.stdout).trim().to_owned());
    }
    for reference in refs {
        if let Ok(output) = Command::new("git").args(["rev-parse", "--git-path", &reference]).output()
            && output.status.success()
        {
            println!("cargo:rerun-if-changed={}", String::from_utf8_lossy(&output.stdout).trim());
        }
    }
    let commit = std::env::var("SOLIDDRAFT_COMMIT")
        .ok()
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| String::from_utf8(o.stdout).ok())
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=SOLIDDRAFT_COMMIT={}", commit.trim());
}
