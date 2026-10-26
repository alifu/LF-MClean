//! Xcode, simulators, Swift Package Manager and `CocoaPods`.

use super::{Category, Filter, Kind, Safety, Source, Target};

const DERIVED_DATA: &str = "Library/Developer/Xcode/DerivedData";
const ARCHIVES: &str = "Library/Developer/Xcode/Archives";
const SWIFTPM: &str = "Library/Caches/org.swift.swiftpm";
const COCOAPODS: &str = "Library/Caches/CocoaPods";

pub(super) const TARGETS: &[Target] = &[
    Target {
        id: "xcode.derived-data",
        name: "DerivedData",
        category: Category::Xcode,
        safety: Safety::Safe,
        effect: "The next build of each project is slower.",
        kind: Kind::Paths,
        sources: &[Source::children(DERIVED_DATA, Filter::Any)],
    },
    Target {
        id: "xcode.device-support",
        name: "Device support files",
        category: Category::Xcode,
        safety: Safety::Safe,
        effect: "Made again the next time a device with that OS version is connected.",
        kind: Kind::Paths,
        sources: &[
            Source::children(
                "Library/Developer/Xcode/iOS DeviceSupport",
                Filter::DirsOnly,
            ),
            Source::children(
                "Library/Developer/Xcode/watchOS DeviceSupport",
                Filter::DirsOnly,
            ),
            Source::children(
                "Library/Developer/Xcode/tvOS DeviceSupport",
                Filter::DirsOnly,
            ),
            Source::children(
                "Library/Developer/Xcode/visionOS DeviceSupport",
                Filter::DirsOnly,
            ),
        ],
    },
    Target {
        id: "xcode.previews",
        name: "SwiftUI preview data",
        category: Category::Xcode,
        safety: Safety::Safe,
        effect: "Previews are built again.",
        kind: Kind::Paths,
        sources: &[Source::children(
            "Library/Developer/Xcode/UserData/Previews",
            Filter::Any,
        )],
    },
    Target {
        id: "xcode.sim-caches",
        name: "Simulator caches",
        category: Category::Xcode,
        safety: Safety::Safe,
        effect: "Made again the next time a simulator boots.",
        kind: Kind::Paths,
        sources: &[Source::children(
            "Library/Developer/CoreSimulator/Caches",
            Filter::Any,
        )],
    },
    Target {
        id: "xcode.sim-unavailable",
        name: "Unavailable simulators",
        category: Category::Xcode,
        safety: Safety::Safe,
        effect: "Only simulators whose runtime is gone are removed, with `simctl`.",
        kind: Kind::UnavailableSimulators,
        sources: &[],
    },
    Target {
        id: "xcode.archives",
        name: "Archives",
        category: Category::Xcode,
        safety: Safety::Risky,
        effect: "Crash reports for these builds can no longer be symbolicated: the dSYMs are inside.",
        kind: Kind::Paths,
        sources: &[Source::nested(
            ARCHIVES,
            "",
            2,
            Filter::Suffix(".xcarchive"),
        )],
    },
    Target {
        id: "spm.repositories",
        name: "Swift package clones",
        category: Category::Xcode,
        safety: Safety::Safe,
        effect: "Cloned again the next time a project needs the package.",
        kind: Kind::Paths,
        sources: &[Source::nested(SWIFTPM, "repositories", 1, Filter::DirsOnly)],
    },
    Target {
        id: "spm.manifests",
        name: "Swift package manifests",
        category: Category::Xcode,
        safety: Safety::Safe,
        effect: "Built again.",
        kind: Kind::Paths,
        sources: &[Source::children(
            SWIFTPM,
            Filter::Named(&["manifests", "package-metadata"]),
        )],
    },
    Target {
        id: "pods.cache",
        name: "CocoaPods cache",
        category: Category::Xcode,
        safety: Safety::Safe,
        effect: "Downloaded again by the next `pod install`.",
        kind: Kind::Paths,
        sources: &[
            Source::nested(COCOAPODS, "Pods/Release", 2, Filter::DirsOnly),
            Source::nested(COCOAPODS, "Pods/External", 2, Filter::DirsOnly),
        ],
    },
    Target {
        id: "pods.specs",
        name: "CocoaPods spec repos",
        category: Category::Xcode,
        safety: Safety::Caution,
        effect: "Cloned again by `pod install`, which can take several minutes.",
        kind: Kind::Paths,
        sources: &[Source::children(".cocoapods/repos", Filter::DirsOnly)],
    },
];

/// The packages Xcode checked out for one project, inside its `DerivedData` folder. Offered only
/// in the package browser: cleaning `DerivedData` already removes them.
pub(super) const SOURCE_PACKAGES: Target = Target {
    id: "spm.source-packages",
    name: "Swift packages of a project",
    category: Category::Xcode,
    safety: Safety::Safe,
    effect: "Xcode resolves and checks the packages out again.",
    kind: Kind::Paths,
    sources: &[Source::nested(
        DERIVED_DATA,
        "",
        2,
        Filter::Named(&["SourcePackages"]),
    )],
};
