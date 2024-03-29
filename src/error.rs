use std::borrow::Cow;
use std::env;
use std::sync::OnceLock;

pub(crate) static APP_NAME: OnceLock<String> = OnceLock::new();

/// Flag if error required
pub(crate) static VERBOSE_ERRORS: OnceLock<bool> = OnceLock::new();

#[derive(Debug)]
pub(crate) enum Error {
    Io(std::io::Error),
    Git(git2::Error),
    Message(Cow<'static, str>),
}

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<git2::Error> for Error {
    fn from(err: git2::Error) -> Self {
        Self::Git(err)
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Self::Message(s.into())
    }
}

impl From<&'static str> for Error {
    fn from(s: &'static str) -> Self {
        Self::Message(s.into())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(err) => Some(err),
            Error::Git(err) => Some(err),
            Error::Message(_) => None,
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => err.fmt(f),
            Error::Git(err) => err.fmt(f),
            Error::Message(err) => err.fmt(f),
        }
    }
}

pub trait LogError {
    fn log(&self);
}

impl<T> LogError for T
where
    T: std::error::Error,
{
    /// Prints the error message to the console.
    ///
    /// Example:
    /// ```ignore
    /// use error::LogError;
    ///
    /// err.log();
    /// ```
    fn log(&self) {
        if *VERBOSE_ERRORS.get().unwrap() {
            let app_name = APP_NAME.get().unwrap();
            eprintln!("{app_name}:  {self:}");
        }
    }
}

pub trait MapLog<T> {
    fn ok_or_log(self) -> Option<T>;
}

impl<T, E> MapLog<T> for Result<T, E>
where
    E: std::error::Error,
{
    /// Prints the error message to the console if result is an error.
    /// Works ordinary as `map_err` function with print.
    ///
    /// Returns an option for ok result.
    ///
    /// Example:
    /// ```ignore
    /// use error::MapLog;
    ///
    /// result.map_log();
    /// ```
    fn ok_or_log(self) -> Option<T> {
        let _ = self.as_ref().map_err(|err| {
            err.log();
        });
        self.ok()
    }
}

pub(crate) fn setup_errors(error_output: bool) {
    let _ = VERBOSE_ERRORS.get_or_init(|| error_output);

    let _ = APP_NAME.get_or_init(|| {
        if error_output {
            env::current_exe()
                .map_or_else(
                    |_| Some(env!("CARGO_BIN_NAME").to_string()),
                    |p| p.file_stem().map(|s| s.to_string_lossy().to_string()),
                )
                .expect("filename by env")
        } else {
            "".to_string()
        }
    });
}
