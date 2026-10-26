//! Gradle and Kotlin/Native.

use super::{Category, Filter, Kind, Safety, Source, Target};

const KONAN: &str = ".konan";

pub(super) const TARGETS: &[Target] = &[
    Target {
        id: "gradle.caches",
        name: "Gradle caches",
        category: Category::GradleKotlin,
        safety: Safety::Safe,
        effect: "Dependencies are downloaded again; offline builds fail until then.",
        kind: Kind::Paths,
        sources: &[Source::children(".gradle/caches", Filter::Any)],
    },
    Target {
        id: "gradle.daemon",
        name: "Gradle daemon logs",
        category: Category::GradleKotlin,
        safety: Safety::Safe,
        effect: "Old logs are gone.",
        kind: Kind::Paths,
        sources: &[Source::nested(
            ".gradle/daemon",
            "",
            2,
            Filter::Suffix(".log"),
        )],
    },
    Target {
        id: "gradle.wrapper",
        name: "Gradle distributions",
        category: Category::GradleKotlin,
        safety: Safety::Caution,
        effect: "That Gradle version is downloaded again by the next build that uses it.",
        kind: Kind::Paths,
        sources: &[Source::children(".gradle/wrapper/dists", Filter::DirsOnly)],
    },
    Target {
        id: "konan.toolchains",
        name: "Kotlin/Native toolchains",
        category: Category::GradleKotlin,
        safety: Safety::Caution,
        effect: "Downloaded again by the next build that uses that version.",
        kind: Kind::Paths,
        sources: &[Source::children(
            KONAN,
            Filter::Prefix("kotlin-native-prebuilt-"),
        )],
    },
    Target {
        id: "konan.dependencies",
        name: "Kotlin/Native dependencies",
        category: Category::GradleKotlin,
        safety: Safety::Caution,
        effect: "Downloaded again (LLVM and sysroots are large) by the next Kotlin/Native build.",
        kind: Kind::Paths,
        sources: &[Source::children(KONAN, Filter::Named(&["dependencies"]))],
    },
];
