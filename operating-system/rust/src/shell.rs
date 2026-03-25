//! Shell command parsing: pipes, redirects, backgrounding.

/// A redirect specification.
#[derive(Debug, Clone)]
pub enum Redirect {
    /// Redirect to/from a file. `append` controls >> vs >.
    File { path: String, append: bool },
    /// Merge with another FD, e.g., 2>&1.
    MergeWith(i32),
}

/// One stage of a pipeline.
#[derive(Debug, Clone)]
pub struct PipelineStage {
    pub command: String,
    pub args: Vec<String>,
    pub stdin_redirect: Option<Redirect>,
    pub stdout_redirect: Option<Redirect>,
    pub stderr_redirect: Option<Redirect>,
}

/// A parsed command line.
#[derive(Debug)]
pub struct Pipeline {
    pub stages: Vec<PipelineStage>,
    pub background: bool,
}

/// Parse a command line into a Pipeline.
pub fn parse_pipeline(input: &str) -> Pipeline {
    let input = input.trim();

    // Check for trailing &
    let (input, background) = if input.ends_with('&') {
        (input[..input.len() - 1].trim(), true)
    } else {
        (input, false)
    };

    // Split on pipes (but not inside quotes)
    let pipe_segments = split_on_pipes(input);

    let mut stages = Vec::new();
    for segment in pipe_segments {
        stages.push(parse_stage(segment.trim()));
    }

    Pipeline { stages, background }
}

/// Split a command line on unquoted `|` characters.
fn split_on_pipes(input: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'|' if !in_single_quote && !in_double_quote => {
                segments.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    segments.push(&input[start..]);
    segments
}

/// Parse a single pipeline stage (one command with its redirects).
fn parse_stage(input: &str) -> PipelineStage {
    let tokens = tokenize(input);

    let mut command = String::new();
    let mut args = Vec::new();
    let mut stdin_redirect = None;
    let mut stdout_redirect = None;
    let mut stderr_redirect = None;

    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];

        if token == ">" || token == ">>" {
            // stdout redirect
            let append = token == ">>";
            if i + 1 < tokens.len() {
                stdout_redirect = Some(Redirect::File {
                    path: tokens[i + 1].clone(),
                    append,
                });
                i += 2;
                continue;
            }
        } else if token == "<" {
            // stdin redirect
            if i + 1 < tokens.len() {
                stdin_redirect = Some(Redirect::File {
                    path: tokens[i + 1].clone(),
                    append: false,
                });
                i += 2;
                continue;
            }
        } else if token == "2>" {
            // stderr redirect
            if i + 1 < tokens.len() {
                stderr_redirect = Some(Redirect::File {
                    path: tokens[i + 1].clone(),
                    append: false,
                });
                i += 2;
                continue;
            }
        } else if token == "2>&1" {
            stderr_redirect = Some(Redirect::MergeWith(1));
            i += 1;
            continue;
        } else if command.is_empty() {
            command = token.clone();
        } else {
            args.push(token.clone());
        }

        i += 1;
    }

    PipelineStage {
        command,
        args,
        stdin_redirect,
        stdout_redirect,
        stderr_redirect,
    }
}

/// Tokenize a command string, handling quoted strings.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            '>' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    // Check for "2>" pattern
                    if current == "2" {
                        current.push('>');
                        // Check for "2>&1"
                        if chars.peek() == Some(&'&') {
                            chars.next(); // consume '&'
                            if chars.peek() == Some(&'1') {
                                chars.next(); // consume '1'
                                current.push('&');
                                current.push('1');
                                tokens.push(current.clone());
                                current.clear();
                                continue;
                            }
                        }
                        tokens.push(current.clone());
                        current.clear();
                        continue;
                    }
                    tokens.push(current.clone());
                    current.clear();
                }
                // Check for >>
                if chars.peek() == Some(&'>') {
                    chars.next();
                    tokens.push(">>".to_string());
                } else {
                    tokens.push(">".to_string());
                }
            }
            '<' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push("<".to_string());
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let p = parse_pipeline("echo hello world");
        assert_eq!(p.stages.len(), 1);
        assert_eq!(p.stages[0].command, "echo");
        assert_eq!(p.stages[0].args, vec!["hello", "world"]);
        assert!(!p.background);
    }

    #[test]
    fn test_pipe() {
        let p = parse_pipeline("cat file.txt | grep hello");
        assert_eq!(p.stages.len(), 2);
        assert_eq!(p.stages[0].command, "cat");
        assert_eq!(p.stages[1].command, "grep");
        assert_eq!(p.stages[1].args, vec!["hello"]);
    }

    #[test]
    fn test_redirect_out() {
        let p = parse_pipeline("echo test > out.txt");
        assert_eq!(p.stages.len(), 1);
        assert_eq!(p.stages[0].command, "echo");
        assert!(matches!(
            &p.stages[0].stdout_redirect,
            Some(Redirect::File {
                path,
                append: false
            }) if path == "out.txt"
        ));
    }

    #[test]
    fn test_redirect_append() {
        let p = parse_pipeline("echo test >> out.txt");
        assert!(matches!(
            &p.stages[0].stdout_redirect,
            Some(Redirect::File { path, append: true }) if path == "out.txt"
        ));
    }

    #[test]
    fn test_redirect_in() {
        let p = parse_pipeline("cat < input.txt");
        assert!(matches!(
            &p.stages[0].stdin_redirect,
            Some(Redirect::File { path, .. }) if path == "input.txt"
        ));
    }

    #[test]
    fn test_background() {
        let p = parse_pipeline("httpd 8080 &");
        assert!(p.background);
        assert_eq!(p.stages[0].command, "httpd");
        assert_eq!(p.stages[0].args, vec!["8080"]);
    }

    #[test]
    fn test_multi_pipe() {
        let p = parse_pipeline("cat file | grep test | wc -l");
        assert_eq!(p.stages.len(), 3);
        assert_eq!(p.stages[0].command, "cat");
        assert_eq!(p.stages[1].command, "grep");
        assert_eq!(p.stages[2].command, "wc");
    }

    #[test]
    fn test_stderr_redirect() {
        let p = parse_pipeline("cmd 2>&1");
        assert!(matches!(
            &p.stages[0].stderr_redirect,
            Some(Redirect::MergeWith(1))
        ));
    }

    #[test]
    fn test_quoted_args() {
        let p = parse_pipeline("echo \"hello world\" foo");
        assert_eq!(p.stages[0].args, vec!["hello world", "foo"]);
    }
}
