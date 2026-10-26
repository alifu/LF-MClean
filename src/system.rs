//! What else is running, and how much space is free.
//!
//! Removing a cache while the tool that uses it runs can break a build or a running emulator, so
//! the clean screen warns about the targets each running tool uses.

use std::io;
use std::path::Path;

use crate::command::CommandRunner;

/// A tool whose caches are better left alone while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tool {
    /// Xcode.
    Xcode,
    /// The iOS Simulator app.
    Simulator,
    /// Android Studio.
    AndroidStudio,
    /// An Android emulator.
    Emulator,
    /// A Gradle daemon.
    GradleDaemon,
    /// The Kotlin compile daemon.
    KotlinDaemon,
}

/// How a tool shows in a process command line.
#[derive(Debug, Clone, Copy)]
enum Mark {
    /// In the path of the program itself, not in its arguments, so `grep .../MacOS/Xcode` is not
    /// Xcode. A mark ending in `/` is a folder the program is in; any other ends the program name.
    Program(&'static str),
    /// A word anywhere in the command line, such as the main class given to `java`.
    Argument(&'static str),
}

/// How each tool shows in a process command line, and the targets it uses.
const TOOLS: &[(Tool, Mark, &[&str])] = &[
    (
        Tool::Xcode,
        Mark::Program("/Contents/MacOS/Xcode"),
        &[
            "xcode.derived-data",
            "xcode.previews",
            "spm.repositories",
            "spm.manifests",
            "spm.source-packages",
            "pods.cache",
        ],
    ),
    (
        Tool::Simulator,
        Mark::Program("Simulator.app/Contents/MacOS/Simulator"),
        &["xcode.sim-caches", "xcode.sim-unavailable"],
    ),
    (
        Tool::AndroidStudio,
        Mark::Program("/Contents/MacOS/studio"),
        &["android.studio-caches", "android.studio-logs"],
    ),
    (
        Tool::Emulator,
        Mark::Program("/emulator/qemu/"),
        &["android.avd", "android.system-images"],
    ),
    (
        Tool::GradleDaemon,
        Mark::Argument("org.gradle.launcher.daemon.bootstrap.GradleDaemon"),
        &[
            "gradle.caches",
            "gradle.daemon",
            "gradle.wrapper",
            "konan.toolchains",
            "konan.dependencies",
        ],
    ),
    (
        Tool::KotlinDaemon,
        Mark::Argument("org.jetbrains.kotlin.daemon.KotlinCompileDaemon"),
        &["konan.toolchains", "konan.dependencies"],
    ),
];

impl Tool {
    /// The name shown in a warning.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Xcode => "Xcode",
            Self::Simulator => "Simulator",
            Self::AndroidStudio => "Android Studio",
            Self::Emulator => "An Android emulator",
            Self::GradleDaemon => "A Gradle daemon",
            Self::KotlinDaemon => "The Kotlin daemon",
        }
    }

    /// The targets this tool uses while it runs.
    #[must_use]
    pub fn targets(self) -> &'static [&'static str] {
        TOOLS
            .iter()
            .find(|(tool, ..)| *tool == self)
            .map_or(&[], |(.., targets)| targets)
    }

    /// What to do instead of removing its caches now.
    #[must_use]
    pub fn advice(self) -> &'static str {
        match self {
            Self::GradleDaemon => "stop it first with `./gradlew --stop`",
            Self::KotlinDaemon => "it stops by itself after a while, or with `./gradlew --stop`",
            Self::Emulator => "close the emulator first",
            Self::Xcode | Self::Simulator | Self::AndroidStudio => "quit it first",
        }
    }
}

/// The tools running now, each once, in a fixed order.
///
/// # Errors
/// When `ps` cannot be run; then nothing is known about running tools.
pub fn running_tools(runner: &dyn CommandRunner) -> io::Result<Vec<Tool>> {
    // `ww`: whole command lines, however wide; a cut one could hide a mark.
    let listing = runner.run("ps", &["-axww", "-o", "args="])?;
    let mut found: Vec<Tool> = TOOLS
        .iter()
        .filter(|(_, mark, _)| listing.lines().any(|line| shows(line, *mark)))
        .map(|(tool, ..)| *tool)
        .collect();
    found.sort();
    found.dedup();
    Ok(found)
}

fn shows(line: &str, mark: Mark) -> bool {
    match mark {
        Mark::Program(text) => line.match_indices(text).any(|(at, _)| {
            in_program_path(line, at) && (text.ends_with('/') || ends_word(line, at + text.len()))
        }),
        Mark::Argument(text) => line
            .match_indices(text)
            .any(|(at, _)| ends_word(line, at + text.len())),
    }
}

/// True when `at` is inside the program path that starts the line. A program path may hold
/// spaces (`/Applications/Android Studio.app/...`), so the program ends at the first argument
/// that starts like a path or an option.
fn in_program_path(line: &str, at: usize) -> bool {
    let before = &line[..at];
    line.starts_with('/') && !before.contains(" /") && !before.contains(" -")
}

/// The mark must end a word: `/Contents/MacOS/Xcode` is Xcode, `.../XcodeHelper` is not.
fn ends_word(line: &str, end: usize) -> bool {
    line[end..]
        .chars()
        .next()
        .is_none_or(|c| c == ' ' || c == '\t')
}

/// The running tools that use `target_id`.
#[must_use]
pub fn tools_using(running: &[Tool], target_id: &str) -> Vec<Tool> {
    running
        .iter()
        .copied()
        .filter(|tool| tool.targets().contains(&target_id))
        .collect()
}

/// Space free for the user on the volume holding `path`, in bytes.
///
/// # Errors
/// When the volume cannot be asked.
pub fn free_space(path: &Path) -> io::Result<u64> {
    let stats = rustix::fs::statvfs(path)?;
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeRunner;

    const PS: &str = "\
/Applications/Xcode.app/Contents/MacOS/Xcode
/Applications/Visual Studio Code.app/Contents/MacOS/Code
/Applications/Xcode.app/Contents/SharedFrameworks/XcodeHelper --flag
/usr/bin/java -Xmx2g -cp gradle-launcher.jar org.gradle.launcher.daemon.bootstrap.GradleDaemon 8.10
/Users/me/Library/Android/sdk/emulator/qemu/darwin-aarch64/qemu-system-aarch64 -avd Pixel_8
";

    #[test]
    fn running_tools_are_found_in_the_process_list() {
        let runner = FakeRunner::answering(PS);

        let tools = running_tools(&runner).unwrap();

        assert_eq!(tools, [Tool::Xcode, Tool::Emulator, Tool::GradleDaemon]);
        assert_eq!(
            runner.calls.lock().unwrap()[0],
            ["ps", "-axww", "-o", "args="]
        );
    }

    #[test]
    fn similar_names_are_not_mistaken_for_tools() {
        let runner = FakeRunner::answering(
            "/Applications/Visual Studio Code.app/Contents/MacOS/Code\n\
             /x/Xcode.app/Contents/MacOS/XcodeHelper\n\
             /x/Simulator.app/Contents/MacOS/SimulatorTrampoline\n",
        );

        assert_eq!(running_tools(&runner).unwrap(), []);
    }

    #[test]
    fn android_studio_and_the_simulator_and_kotlin_are_found() {
        let runner = FakeRunner::answering(
            "/Applications/Android Studio.app/Contents/MacOS/studio\n\
             /Applications/Xcode.app/Contents/Developer/Applications/Simulator.app/Contents/MacOS/Simulator\n\
             java -cp k.jar org.jetbrains.kotlin.daemon.KotlinCompileDaemon --daemon-runFilesPath x\n",
        );

        assert_eq!(
            running_tools(&runner).unwrap(),
            [Tool::Simulator, Tool::AndroidStudio, Tool::KotlinDaemon]
        );
    }

    #[test]
    fn a_tool_path_given_as_an_argument_is_not_the_tool_running() {
        let runner = FakeRunner::answering(
            "/usr/bin/grep /Applications/Xcode.app/Contents/MacOS/Xcode\n\
             vim /Applications/Android Studio.app/Contents/MacOS/studio\n\
             /bin/cat /sdk/emulator/qemu/darwin-aarch64/qemu-system-aarch64\n\
             /opt/homebrew/bin/qemu-system-aarch64 -machine virt\n\
             /usr/bin/less -p /x/Simulator.app/Contents/MacOS/Simulator\n",
        );

        assert_eq!(running_tools(&runner).unwrap(), []);
    }

    #[test]
    fn a_program_path_with_spaces_is_still_found() {
        let runner = FakeRunner::answering(
            "/Applications/Xcode 16 beta.app/Contents/MacOS/Xcode -NSDocumentRevisionsDebugMode YES\n",
        );

        assert_eq!(running_tools(&runner).unwrap(), [Tool::Xcode]);
    }

    #[test]
    fn ps_failing_is_an_error() {
        assert!(running_tools(&FakeRunner::failing("no ps")).is_err());
    }

    #[test]
    fn every_tool_names_only_real_targets() {
        for (tool, ..) in TOOLS {
            assert_ne!(tool.targets().len(), 0);
            assert_ne!(tool.advice(), "");
            for id in tool.targets() {
                assert!(crate::catalog::find(id).is_some(), "{id}");
            }
        }
    }

    #[test]
    fn tools_using_a_target_are_picked_out() {
        let running = [Tool::Xcode, Tool::GradleDaemon, Tool::KotlinDaemon];

        assert_eq!(tools_using(&running, "xcode.derived-data"), [Tool::Xcode]);
        assert_eq!(
            tools_using(&running, "konan.toolchains"),
            [Tool::GradleDaemon, Tool::KotlinDaemon]
        );
        assert_eq!(tools_using(&running, "android.avd"), []);
    }

    #[test]
    fn free_space_is_known_for_a_real_folder() {
        assert!(free_space(Path::new("/")).unwrap() > 0);
        assert!(free_space(Path::new("/no/such/folder")).is_err());
    }
}
