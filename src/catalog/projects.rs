//! What `purge` offers inside a project. These are not part of a whole scan: they are found by
//! looking at the projects, not at fixed cache folders.

use super::{Category, Kind, Safety, Target};

pub(super) const TARGETS: &[Target] = &[
    Target {
        id: "purge.build",
        name: "Gradle build folders",
        category: Category::Projects,
        safety: Safety::Safe,
        effect: "The next build compiles the module again.",
        kind: Kind::Paths,
        sources: &[],
    },
    Target {
        id: "purge.build-unsure",
        name: "Build folders with other content",
        category: Category::Projects,
        safety: Safety::Caution,
        effect: "Holds files Gradle does not make and git may not have; they are gone for good.",
        kind: Kind::Paths,
        sources: &[],
    },
    Target {
        id: "purge.gradle",
        name: "Project .gradle folders",
        category: Category::Projects,
        safety: Safety::Safe,
        effect: "Gradle rebuilds its project cache on the next build.",
        kind: Kind::Paths,
        sources: &[],
    },
    Target {
        id: "purge.kotlin",
        name: "Project .kotlin folders",
        category: Category::Projects,
        safety: Safety::Safe,
        effect: "The Kotlin compiler rebuilds its session data.",
        kind: Kind::Paths,
        sources: &[],
    },
    Target {
        id: "purge.ios-build",
        name: "iOS app build folders",
        category: Category::Projects,
        safety: Safety::Safe,
        effect: "Xcode builds the iOS app again.",
        kind: Kind::Paths,
        sources: &[],
    },
    Target {
        id: "purge.pods",
        name: "CocoaPods Pods folders",
        category: Category::Projects,
        safety: Safety::Caution,
        effect: "`pod install` has to run again, which needs the network or the pod cache.",
        kind: Kind::Paths,
        sources: &[],
    },
];
