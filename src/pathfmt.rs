use std::path::Path;

pub fn compact_path(p: &Path, max_chars: usize) -> String {
    let mut s = p.display().to_string();
    if let Some(home) = dirs::home_dir() {
        let home_str = home.display().to_string();
        if s == home_str {
            s = "~".to_string();
        } else if let Some(rest) = s.strip_prefix(&(home_str + "/")) {
            s = format!("~/{rest}");
        }
    }
    if s.chars().count() <= max_chars {
        return s;
    }
    // Keep the tail, elide the head.
    let mut tail = String::new();
    for c in s.chars().rev() {
        if tail.chars().count() + 1 >= max_chars.saturating_sub(2) {
            break;
        }
        tail.insert(0, c);
    }
    format!("…{tail}")
}

pub fn basename(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}
