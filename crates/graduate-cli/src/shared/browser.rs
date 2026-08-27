//! Shared boundary for local browser side effects.

use std::io;

pub(crate) trait BrowserLauncher: Send + Sync {
    fn open(&self, url: &str) -> io::Result<()>;
}

pub(crate) struct SystemBrowserLauncher;

impl BrowserLauncher for SystemBrowserLauncher {
    fn open(&self, url: &str) -> io::Result<()> {
        webbrowser::open(url)
    }
}
