/// Splits a command-line style string: whitespace separates, double quotes group.
pub fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    out
}

pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.is_empty() || a.chars().any(char::is_whitespace) {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_join_round_trip() {
        let args = split_args(r#"--port 42420  "C:\My Data" "" x"#);
        assert_eq!(args, ["--port", "42420", r"C:\My Data", "", "x"]);
        assert_eq!(split_args(&join_args(&args)), args);
    }
}
