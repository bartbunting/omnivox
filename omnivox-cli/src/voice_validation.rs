//! Development-only native validation. No activation or capability advertisement.
#[cfg(any(target_os = "linux", windows))]
mod owned;
#[cfg(any(target_os = "linux", windows))]
mod supported;

pub fn requested(args: &[String]) -> bool {
    args.first().is_some_and(|arg| {
        matches!(
            arg.as_str(),
            "--validate-voice-library" | "--internal-voice-validation-worker"
        )
    })
}

pub fn run(args: &[String]) -> anyhow::Result<()> {
    #[cfg(any(target_os = "linux", windows))]
    {
        supported::run(args)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = args;
        anyhow::bail!("disposable voice validation is currently implemented for Linux and Windows")
    }
}
