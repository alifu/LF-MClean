//! Android Studio, emulators and system images.

use super::{Category, Filter, Kind, Safety, Source, Target};

pub(super) const TARGETS: &[Target] = &[
    Target {
        id: "android.studio-caches",
        name: "Android Studio and JetBrains caches",
        category: Category::Android,
        safety: Safety::Safe,
        effect: "The IDE indexes your projects again.",
        kind: Kind::Paths,
        sources: &[
            Source::children("Library/Caches/Google", Filter::Prefix("AndroidStudio")),
            Source::children("Library/Caches/JetBrains", Filter::Any),
        ],
    },
    Target {
        id: "android.studio-logs",
        name: "Android Studio logs",
        category: Category::Android,
        safety: Safety::Safe,
        effect: "Old logs are gone.",
        kind: Kind::Paths,
        sources: &[Source::children(
            "Library/Logs/Google",
            Filter::Prefix("AndroidStudio"),
        )],
    },
    Target {
        id: "android.avd",
        name: "Emulators",
        category: Category::Android,
        safety: Safety::Risky,
        effect: "The emulator and everything saved inside it (apps, accounts, files) is lost.",
        kind: Kind::Emulators,
        sources: &[Source::children(".android/avd", Filter::Suffix(".avd"))],
    },
    Target {
        id: "android.system-images",
        name: "Emulator system images",
        category: Category::Android,
        safety: Safety::Risky,
        effect: "Emulators that use the image stop working until it is downloaded again.",
        kind: Kind::Paths,
        sources: &[Source::nested(
            "Library/Android/sdk/system-images",
            "",
            3,
            Filter::DirsOnly,
        )],
    },
];
