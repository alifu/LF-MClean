//! A scan report as text for the terminal, or as JSON for scripts.

use chrono::{DateTime, FixedOffset};
use serde::Serialize;

use super::{Item, Removal, Report, Skipped, TargetReport};
use crate::catalog::{Category, Safety};
use crate::format::{age, human_size, printable};

const SAFETY_WIDTH: usize = 7;
const COLUMN_GAP: &str = "  ";

/// The report as a table grouped by category. Targets with nothing to remove are left out; with
/// `with_items` each item is listed under its target.
#[must_use]
pub fn table(report: &Report, now: DateTime<FixedOffset>, with_items: bool) -> String {
    let shown: Vec<&TargetReport> = report
        .targets
        .iter()
        .filter(|target| !target.items.is_empty())
        .collect();
    let mut text = if shown.is_empty() {
        "Nothing found to clean.\n".to_string()
    } else {
        grouped(&shown, report, now, with_items)
    };
    text.push_str(&not_offered(report));
    text
}

/// One heading and its rows per category, then the total.
fn grouped(
    shown: &[&TargetReport],
    report: &Report,
    now: DateTime<FixedOffset>,
    with_items: bool,
) -> String {
    let groups = by_category(shown);
    let rows: Vec<Row> = groups
        .iter()
        .flat_map(|(_, targets)| targets.iter().flat_map(|t| rows_for(t, now, with_items)))
        .collect();
    let label_width = rows.iter().map(|row| row.label.len()).max().unwrap_or(0);
    let size_width = rows.iter().map(|row| row.size.len()).max().unwrap_or(0);
    let mut text = String::new();
    for (category, targets) in &groups {
        text.push_str(category.title());
        text.push('\n');
        for row in targets.iter().flat_map(|t| rows_for(t, now, with_items)) {
            let line = format!(
                "{:<label_width$}{COLUMN_GAP}{:>size_width$}{COLUMN_GAP}{}",
                row.label, row.size, row.rest
            );
            text.push_str(line.trim_end());
            text.push('\n');
        }
    }
    let partial = report
        .targets
        .iter()
        .flat_map(|t| &t.items)
        .any(|i| i.partial);
    text.push_str("\nTotal: ");
    text.push_str(&size_text(report.total_bytes(), partial));
    text.push('\n');
    text
}

struct Row {
    label: String,
    size: String,
    rest: String,
}

fn rows_for(target: &TargetReport, now: DateTime<FixedOffset>, with_items: bool) -> Vec<Row> {
    let count = target.items.len();
    let partial = target.items.iter().any(|item| item.partial);
    let head = Row {
        label: format!("  {}", target.target.name),
        size: size_text(target.total_bytes(), partial),
        rest: format!(
            "{:<SAFETY_WIDTH$}{COLUMN_GAP}{count} {}",
            target.target.safety.label(),
            if count == 1 { "item" } else { "items" }
        ),
    };
    let items = target.items.iter().filter(|_| with_items).map(|item| Row {
        label: format!("    {}", printable(&item.name)),
        size: size_text(item.size_bytes, item.partial),
        rest: item
            .modified
            .map_or_else(String::new, |time| age(time, now)),
    });
    std::iter::once(head).chain(items).collect()
}

/// Groups in catalog order; a target list is never empty.
fn by_category<'a>(shown: &[&'a TargetReport]) -> Vec<(Category, Vec<&'a TargetReport>)> {
    let mut groups: Vec<(Category, Vec<&TargetReport>)> = Vec::new();
    for target in shown {
        let category = target.target.category;
        match groups.last_mut() {
            Some((last, targets)) if *last == category => targets.push(target),
            _ => groups.push((category, vec![target])),
        }
    }
    groups
}

/// A `+` marks a size that is a lower bound because some folders could not be read.
fn size_text(bytes: u64, partial: bool) -> String {
    let plus = if partial { "+" } else { "" };
    format!("{}{plus}", human_size(bytes))
}

fn not_offered(report: &Report) -> String {
    let skipped: Vec<(&TargetReport, &Skipped)> = report
        .targets
        .iter()
        .flat_map(|target| target.skipped.iter().map(move |skip| (target, skip)))
        .collect();
    if skipped.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = skipped
        .iter()
        .map(|(target, skip)| format!("  {}: {}\n", target.target.name, printable(&skip.reason)))
        .collect();
    format!("\nNot offered ({}):\n{}", skipped.len(), lines.concat())
}

/// The report as pretty-printed JSON, with every item's paths.
///
/// # Errors
/// When serialization fails, which no report should cause.
pub fn json(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&ReportJson::from(report))
}

#[derive(Serialize)]
struct ReportJson<'a> {
    total_bytes: u64,
    targets: Vec<TargetJson<'a>>,
}

#[derive(Serialize)]
struct TargetJson<'a> {
    id: &'static str,
    name: &'static str,
    category: Category,
    safety: Safety,
    effect: &'static str,
    total_bytes: u64,
    items: Vec<ItemJson<'a>>,
    skipped: Vec<SkippedJson<'a>>,
}

#[derive(Serialize)]
struct ItemJson<'a> {
    name: &'a str,
    size_bytes: u64,
    partial: bool,
    modified: Option<DateTime<FixedOffset>>,
    removal: &'static str,
    paths: Vec<String>,
    udids: Vec<String>,
}

#[derive(Serialize)]
struct SkippedJson<'a> {
    path: Option<String>,
    reason: &'a str,
}

impl<'a> From<&'a Report> for ReportJson<'a> {
    fn from(report: &'a Report) -> Self {
        Self {
            total_bytes: report.total_bytes(),
            targets: report.targets.iter().map(TargetJson::from).collect(),
        }
    }
}

impl<'a> From<&'a TargetReport> for TargetJson<'a> {
    fn from(report: &'a TargetReport) -> Self {
        let target = report.target;
        Self {
            id: target.id,
            name: target.name,
            category: target.category,
            safety: target.safety,
            effect: target.effect,
            total_bytes: report.total_bytes(),
            items: report.items.iter().map(ItemJson::from).collect(),
            skipped: report
                .skipped
                .iter()
                .map(|skip| SkippedJson {
                    path: skip.path.as_ref().map(|p| p.display().to_string()),
                    reason: &skip.reason,
                })
                .collect(),
        }
    }
}

impl<'a> From<&'a Item> for ItemJson<'a> {
    fn from(item: &'a Item) -> Self {
        let (removal, paths, udids) = match &item.removal {
            Removal::Paths { .. } => (
                "paths",
                item.removal
                    .paths()
                    .iter()
                    .map(|p| p.path().display().to_string())
                    .collect(),
                Vec::new(),
            ),
            Removal::SimctlDelete { .. } => (
                "simctl-delete",
                Vec::new(),
                item.removal
                    .udids()
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
        };
        Self {
            name: &item.name,
            size_bytes: item.size_bytes,
            partial: item.partial,
            modified: item.modified,
            removal,
            paths,
            udids,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::catalog::{self, Simulator};

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(7 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 10, 26, 12, 0, 0)
            .unwrap()
    }

    fn item(target: &'static str, name: &str, bytes: u64, days_old: i64, partial: bool) -> Item {
        Item {
            target,
            name: name.to_string(),
            removal: Removal::SimctlDelete {
                simulators: vec![Simulator {
                    udid: "AAAA".to_string(),
                    folder: "/Users/x/Library/Developer/CoreSimulator/Devices/AAAA".into(),
                }],
            },
            size_bytes: bytes,
            partial,
            modified: Some(now() - Duration::days(days_old)),
        }
    }

    fn target_report(id: &str, items: Vec<Item>, skipped: Vec<Skipped>) -> TargetReport {
        TargetReport {
            target: catalog::find(id).unwrap(),
            items,
            skipped,
        }
    }

    fn sample() -> Report {
        Report {
            targets: vec![
                target_report(
                    "xcode.derived-data",
                    vec![
                        item("xcode.derived-data", "App-abc", 1_500_000_000, 3, false),
                        item("xcode.derived-data", "Lib-def", 400_000_000, 210, false),
                    ],
                    vec![],
                ),
                target_report("xcode.previews", vec![], vec![]),
                target_report(
                    "android.avd",
                    vec![item("android.avd", "Medium_Phone", 8_200_000_000, 14, true)],
                    vec![],
                ),
            ],
        }
    }

    #[test]
    fn the_table_groups_by_category_and_leaves_out_empty_targets() {
        let text = table(&sample(), now(), false);

        let expected = "\
Xcode
  DerivedData   1.9 GB  safe     2 items
Android
  Emulators    8.2 GB+  risky    1 item

Total: 10.1 GB+
";
        assert_eq!(text, expected);
    }

    #[test]
    fn with_items_each_item_is_listed_with_its_age() {
        let text = table(&sample(), now(), true);

        let expected = "\
Xcode
  DerivedData       1.9 GB  safe     2 items
    App-abc         1.5 GB  3 d ago
    Lib-def       400.0 MB  7 mo ago
Android
  Emulators        8.2 GB+  risky    1 item
    Medium_Phone   8.2 GB+  2 w ago

Total: 10.1 GB+
";
        assert_eq!(text, expected);
    }

    #[test]
    fn an_empty_report_says_so() {
        let report = Report {
            targets: vec![target_report("gradle.caches", vec![], vec![])],
        };

        assert_eq!(table(&report, now(), false), "Nothing found to clean.\n");
    }

    #[test]
    fn skipped_things_are_listed_with_their_reason() {
        let report = Report {
            targets: vec![
                target_report(
                    "gradle.caches",
                    vec![],
                    vec![Skipped {
                        target: "gradle.caches",
                        path: Some("/Users/x/.gradle/caches/odd".into()),
                        reason: "/Users/x/.gradle/caches/odd belongs to another user".into(),
                    }],
                ),
                target_report(
                    "xcode.sim-unavailable",
                    vec![],
                    vec![Skipped {
                        target: "xcode.sim-unavailable",
                        path: None,
                        reason: "simctl could not be run: no Xcode".into(),
                    }],
                ),
            ],
        };

        let text = table(&report, now(), false);

        assert!(text.starts_with("Nothing found to clean.\n"), "{text}");
        assert!(text.contains("Not offered (2):"), "{text}");
        assert!(
            text.contains("  Gradle caches: /Users/x/.gradle/caches/odd belongs to another user")
        );
        assert!(text.contains("  Unavailable simulators: simctl could not be run: no Xcode"));
    }

    #[test]
    fn json_has_totals_targets_items_and_skips() {
        let report = sample();

        let value: serde_json::Value = serde_json::from_str(&json(&report).unwrap()).unwrap();

        assert_eq!(value["total_bytes"], 10_100_000_000_u64);
        let first = &value["targets"][0];
        assert_eq!(first["id"], "xcode.derived-data");
        assert_eq!(first["category"], "xcode");
        assert_eq!(first["safety"], "safe");
        assert_eq!(first["total_bytes"], 1_900_000_000_u64);
        assert_eq!(first["items"][0]["name"], "App-abc");
        assert_eq!(first["items"][0]["removal"], "simctl-delete");
        assert_eq!(first["items"][0]["udids"], serde_json::json!(["AAAA"]));
        assert_eq!(first["items"][0]["paths"], serde_json::json!([]));
        assert_eq!(first["items"][0]["partial"], false);
        assert_eq!(value["targets"][2]["items"][0]["partial"], true);
        assert_eq!(value["targets"][1]["items"], serde_json::json!([]));
        assert!(
            first["items"][0]["modified"]
                .as_str()
                .unwrap()
                .starts_with("2026-10-2")
        );
    }
}
