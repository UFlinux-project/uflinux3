//! Shared, side-effect-free validation for the installer rewrite.
//! Storage operations must only run after the remaining preflight checks succeed.
use std::path::Path;

pub fn valid_user(user: &str) -> bool {
    let bytes = user.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 31
        && (bytes[0].is_ascii_lowercase() || bytes[0] == b'_')
        && bytes[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
        && !matches!(user, "root" | "live" | "greetd")
}

pub fn valid_hostname(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

pub fn valid_password(password: &str) -> bool {
    let n = password.chars().count();
    (8..=1024).contains(&n) && !password.chars().any(|c| matches!(c, ':' | '\n' | '\r' | '\0'))
}

/// Layout entries and boot/root partition numbers for automatic installation.
pub fn storage_layout(arch: &str, mode: &str) -> Option<(&'static [&'static str], u32, u32)> {
    match (arch, mode) {
        ("x86" | "x86_64" | "aarch64" | "armv7" | "riscv64", "uefi") =>
            Some((&["size=1G,type=U", "type=L"], 1, 2)),
        ("x86" | "x86_64", "bios") =>
            Some((&["size=1G,type=L,attrs=LegacyBIOSBootable", "type=L"], 1, 2)),
        ("ppc64le", "native") => Some((&["size=8M,type=9e1a2d38-c612-4316-aa26-8b49521e5a8b", "size=1G,type=L", "type=L"], 2, 3)),
        ("s390x", "native") => Some((&["size=1G,type=L", "type=L"], 1, 2)),
        _ => None,
    }
}

pub fn partition_path(disk: &str, number: u32) -> String {
    format!("{disk}{}{number}", if disk.as_bytes().last().is_some_and(u8::is_ascii_digit) { "p" } else { "" })
}

/// The target must not be one of the live system's protected roots.
pub fn protected_target(target: &Path) -> bool {
    ["/", "/boot", "/home", "/etc", "/usr", "/var"].iter().any(|p| target == Path::new(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_names_reject_reserved_and_unsafe_input() {
        for name in ["", "root", "live", "greetd", "Alice", "-alice", "alice:bad", "alice\n", "é"] { assert!(!valid_user(name), "{name:?}"); }
        for name in ["alice", "_system-user", "a9"] { assert!(valid_user(name)); }
        assert!(!valid_user(&"a".repeat(32)));
    }
    #[test]
    fn hostname_and_password_validation() {
        assert!(valid_hostname("uflinux-1"));
        for name in ["", "-bad", "UPPER", "a.b", "a\n"] { assert!(!valid_hostname(name)); }
        for password in ["short", "bad:password", "bad\npassword", "bad\rpassword", "bad\0password"] { assert!(!valid_password(password)); }
        assert!(valid_password("correct horse battery staple"));
    }
    #[test]
    fn firmware_layouts_and_partition_names() {
        for arch in ["x86", "x86_64", "aarch64", "armv7", "riscv64"] { assert_eq!(storage_layout(arch, "uefi").unwrap().1, 1); }
        assert_eq!(storage_layout("ppc64le", "native").unwrap().1, 2);
        assert!(storage_layout("aarch64", "native").is_none());
        assert_eq!(partition_path("/dev/vda", 1), "/dev/vda1");
        assert_eq!(partition_path("/dev/nvme0n1", 2), "/dev/nvme0n1p2");
    }
    #[test]
    fn protected_roots_are_rejected() {
        for path in ["/", "/boot", "/home", "/etc", "/usr", "/var"] { assert!(protected_target(Path::new(path))); }
        assert!(!protected_target(Path::new("/mnt")));
    }
}
