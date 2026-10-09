//! tubetui — YouTube in your terminal. Early development.

fn banner() -> String {
    format!(
        "{} {} — work in progress, see {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY")
    )
}

fn main() {
    println!("{}", banner());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_shows_name_and_version() {
        let b = banner();
        assert!(b.starts_with("tubetui "));
        assert!(b.contains(env!("CARGO_PKG_VERSION")));
    }
}
