//! The shim launcher and its `.shim` file format.
//!
//! A shim is a copy of `wpk-shim.exe` (console) or `wpk-shimw.exe` (gui)
//! named after the command it provides. Next to it, a `.shim` file with the
//! same stem names the target:
//!
//! ```text
//! path = C:\Users\x\wpk\opt\fd\10.5.2\fd.exe
//! args = --hidden
//! ```
//!
//! `args` is optional and goes before the arguments the shim was called with.
//! `dir`, also optional, is the working directory of the target; without it
//! the target inherits the shim's. `arg0`, also optional, replaces the path
//! as the target's `argv[0]`, for multi-call programs that act on the name
//! they were started as.

#[cfg(windows)]
mod launch;

/// Which shim binary is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// Waits for the target and returns its exit code.
    Console,
    /// Starts the target and exits.
    Gui,
}

/// Contents of a `.shim` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShimFile {
    pub path: String,
    pub args: Option<String>,
    pub dir: Option<String>,
    pub arg0: Option<String>,
}

impl ShimFile {
    pub fn parse(text: &str) -> Result<ShimFile, String> {
        let mut path = None;
        let mut args = None;
        let mut dir = None;
        let mut arg0 = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(format!("invalid line: {line}"));
            };
            let value = Some(value.trim().to_owned()).filter(|v| !v.is_empty());
            match key.trim() {
                "path" => path = value,
                "args" => args = value,
                "dir" => dir = value,
                "arg0" => arg0 = value,
                key => return Err(format!("unknown key: {key}")),
            }
        }
        let path = path.ok_or("missing path")?;
        Ok(ShimFile { path, args, dir, arg0 })
    }

    pub fn render(&self) -> String {
        let mut text = format!("path = {}\n", self.path);
        if let Some(args) = &self.args {
            text.push_str(&format!("args = {args}\n"));
        }
        if let Some(dir) = &self.dir {
            text.push_str(&format!("dir = {dir}\n"));
        }
        if let Some(arg0) = &self.arg0 {
            text.push_str(&format!("arg0 = {arg0}\n"));
        }
        text
    }

    /// Command line for the target: the quoted `arg0` or path, `args`, then
    /// `rest`, the shim's own arguments as it received them.
    pub fn command_line(&self, rest: &[u16]) -> Vec<u16> {
        let first = self.arg0.as_deref().unwrap_or(&self.path);
        let mut line: Vec<u16> = format!("\"{first}\"").encode_utf16().collect();
        if let Some(args) = &self.args {
            line.push(b' ' as u16);
            line.extend(args.encode_utf16());
        }
        if !rest.is_empty() {
            line.push(b' ' as u16);
            line.extend_from_slice(rest);
        }
        line
    }
}

/// The arguments of a Windows command line after the program name, unparsed.
///
/// The program name ends at the first space or tab outside double quotes, as
/// the C runtime splits `argv[0]`; quotes in it cannot be escaped.
pub fn skip_program_name(line: &[u16]) -> &[u16] {
    let is_blank = |c: &u16| *c == b' ' as u16 || *c == b'\t' as u16;
    let mut in_quotes = false;
    let mut end = line.len();
    for (i, c) in line.iter().enumerate() {
        if *c == b'"' as u16 {
            in_quotes = !in_quotes;
        } else if !in_quotes && is_blank(c) {
            end = i;
            break;
        }
    }
    let rest = &line[end..];
    let start = rest.iter().position(|c| !is_blank(c)).unwrap_or(rest.len());
    &rest[start..]
}

/// Run the target of this shim and return the exit code for the shim.
#[cfg(windows)]
pub fn run(variant: Variant) -> i32 {
    match launch::run(variant) {
        Ok(code) => code,
        Err(message) => {
            launch::report(variant, &format!("wpk-shim: {message}"));
            1
        }
    }
}

#[cfg(not(windows))]
pub fn run(_variant: Variant) -> i32 {
    eprintln!("wpk-shim: only runs on Windows");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn parse_and_render() {
        let shim =
            ShimFile::parse("path = C:\\a b\\x.exe\r\nargs = -c \"d e\"\ndir = C:\\a b\n").unwrap();
        assert_eq!(shim.path, "C:\\a b\\x.exe");
        assert_eq!(shim.args.as_deref(), Some("-c \"d e\""));
        assert_eq!(shim.dir.as_deref(), Some("C:\\a b"));
        assert_eq!(ShimFile::parse(&shim.render()).unwrap(), shim);
    }

    #[test]
    fn parse_rejects_missing_path_and_unknown_key() {
        assert!(ShimFile::parse("args = x").is_err());
        assert!(ShimFile::parse("path = x\nfoo = y").is_err());
    }

    #[test]
    fn skip_program_name_cases() {
        assert_eq!(
            skip_program_name(&wide("\"C:\\a b\\fd.exe\"  -H x")),
            wide("-H x")
        );
        assert_eq!(skip_program_name(&wide("fd.exe\t-H")), wide("-H"));
        assert_eq!(skip_program_name(&wide("fd")), wide(""));
        assert_eq!(skip_program_name(&wide("\"fd")), wide(""));
        assert_eq!(skip_program_name(&wide("\"a\"b c")), wide("c"));
        assert_eq!(skip_program_name(&wide("\"a b\"")), wide(""));
    }

    #[test]
    fn command_line_order() {
        let shim = ShimFile {
            path: "C:\\x.exe".into(),
            args: Some("--a".into()),
            dir: None,
            arg0: None,
        };
        assert_eq!(shim.command_line(&wide("b")), wide("\"C:\\x.exe\" --a b"));
        let bare = ShimFile {
            path: "x".into(),
            args: None,
            dir: None,
            arg0: Some("cargo".into()),
        };
        assert_eq!(bare.command_line(&[]), wide("\"cargo\""));
        assert_eq!(ShimFile::parse(&bare.render()).unwrap(), bare);
    }
}
