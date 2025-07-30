use std::path::PathBuf;

pub struct JarvisIO {
    base: PathBuf,
}

impl JarvisIO {
    pub fn new() -> Self {
        let base = dirs::home_dir().unwrap().join(".jarvis");
        std::fs::create_dir_all(&base).unwrap();
        Self { base }
    }

    pub fn write_status(&self, status: &str) {
        let _ = std::fs::write(self.base.join("jarvis.status"), status);
    }

    pub fn write_spoken(&self, text: &str) {
        let _ = std::fs::write(self.base.join("jarvis.spoken"), text);
    }

    pub fn write_heard(&self, text: &str) {
        let _ = std::fs::write(self.base.join("jarvis.heard"), text);
    }

    /// Persist the given working directory path for future shell tasks.
    pub fn write_working_directory(&self, path: &str) {
        let _ = std::fs::write(self.base.join("jarvis.working_directory"), path);
    }

    pub fn current_status(&self) -> Option<String> {
        std::fs::read_to_string(self.base.join("jarvis.status")).ok()
    }

    /// Read the persisted working directory, if set.
    pub fn read_working_directory(&self) -> Option<String> {
        std::fs::read_to_string(self.base.join("jarvis.working_directory"))
            .ok()
            .map(|s| s.trim().to_string())
    }

    pub fn cancel_tts(&self) {
        let _ = std::process::Command::new("bash")
            .arg("-c")
            .arg("~/.jarvis/scripts/cancel_tts.sh")
            .spawn();
    }

    pub fn set_pid(&self) {
        let pid = std::process::id().to_string();
        let _ = std::fs::write(self.base.join("jarvis"), pid);
    }
}
