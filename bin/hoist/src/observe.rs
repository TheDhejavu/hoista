//! Watching: `metrics`, `activity`, `billing` and `usage`. Every number
//! here is one a provider reported; Hoista never estimates.

use crate::{
    context::{self, Context},
    ui,
};
use clap::{Args, Subcommand};
use serde_json::{Value, json};

/// The usage range shown when none is given.
const DEFAULT_USAGE_DAYS: i64 = 30;

// hoist metrics

#[derive(Args)]
pub(crate) struct MetricsArgs {
    /// Resource name or id. In a terminal, pick it from a list instead.
    resource: Option<String>,
    /// The resource's project, when naming it by name.
    #[arg(long, short = 'p', value_name = "PROJECT")]
    project: Option<String>,
    /// How far back: 15m, 1h, 6h, 24h, 7d or 30d.
    #[arg(long, default_value = "1h", value_parser = RANGES.map(|(name, _)| name))]
    since: String,
}

/// The ranges `hoist metrics` offers, in seconds.
const RANGES: [(&str, i64); 6] = [
    ("15m", 15 * 60),
    ("1h", 60 * 60),
    ("6h", 6 * 60 * 60),
    ("24h", 24 * 60 * 60),
    ("7d", 7 * 24 * 60 * 60),
    ("30d", 30 * 24 * 60 * 60),
];
/// Points a chart aims for, and its finest step.
const CHART_POINTS: i64 = 60;
const MIN_STEP_SECS: i64 = 60;

fn seconds(range: &str) -> i64 {
    RANGES
        .iter()
        .find(|(name, _)| *name == range)
        .map_or(60 * 60, |(_, seconds)| *seconds)
}

fn number(value: f64, unit: &str) -> String {
    match unit {
        "percent" => format!("{value:.1}%"),
        "milliseconds" => format!("{value:.0} ms"),
        "seconds" => format!("{value:.0} s"),
        "cores" => format!("{value:.2} cores"),
        "bytes" => {
            let units = ["B", "KB", "MB", "GB", "TB"];
            let mut value = value;
            let mut at = 0;
            while value >= 1000.0 && at < units.len() - 1 {
                value /= 1000.0;
                at += 1;
            }
            format!("{value:.1} {}", units[at])
        }
        _ => format!("{value:.0}"),
    }
}

/// A series' headline: added up, averaged or the newest, as the chart says.
fn headline(points: &[Value], summary: &str) -> Option<f64> {
    let values: Vec<f64> = points
        .iter()
        .filter_map(|point| point["value"].as_f64())
        .collect();
    if values.is_empty() {
        return None;
    }
    Some(match summary {
        "total" => values.iter().sum(),
        "latest" => *values.last()?,
        _ => values.iter().sum::<f64>() / values.len() as f64,
    })
}

pub(crate) async fn metrics(context: &Context, args: MetricsArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let resource = context::resource(
        context,
        &client,
        args.resource.as_deref(),
        args.project.as_deref(),
    )
    .await?;
    let span = seconds(&args.since);
    let to = chrono::Utc::now();
    let from = to - chrono::Duration::seconds(span);
    let step = (span / CHART_POINTS).max(MIN_STEP_SECS).to_string();
    let (from, to) = (
        from.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        to.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    let found = client
        .get_json(&format!(
            "{}?{}",
            context::path(&["resources", &ui::text(&resource, "id"), "metrics"]),
            context::query(&[("from", &from), ("to", &to), ("step", &step)])
        ))
        .await?;
    ui::show(&found, |found| {
        println!("{}, last {}", ui::text(&resource, "name"), args.since);
        let mut table = ui::Table::new(&["CHART", "LINE", "VALUE", "HOW"]);
        for chart in found["charts"].as_array().into_iter().flatten() {
            let unit = ui::text(chart, "unit");
            let summary = ui::text(chart, "summary");
            for series in chart["series"].as_array().into_iter().flatten() {
                let points = series["points"].as_array().cloned().unwrap_or_default();
                table.row(vec![
                    ui::text(chart, "label"),
                    ui::text(series, "name"),
                    headline(&points, &summary)
                        .map_or_else(|| "-".into(), |value| number(value, &unit)),
                    summary.clone(),
                ]);
            }
        }
        if table.is_empty() {
            ui::note("The provider reported no metrics for this range.");
        } else {
            table.print();
        }
        if let Some(note) = found["note"].as_str() {
            ui::note(&ui::clean(note));
        }
    })
}

// hoist activity

#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct ActivityArgs {
    #[command(subcommand)]
    command: Option<ActivityCommand>,
    /// Only this project.
    #[arg(long, short = 'p', value_name = "PROJECT")]
    project: Option<String>,
    /// Only this agent's calls (name or id).
    #[arg(long, value_name = "AGENT")]
    agent: Option<String>,
    /// Only calls agents made.
    #[arg(long, conflicts_with = "agent")]
    agents: bool,
    /// How many to show, newest first.
    #[arg(long, short = 'n', default_value_t = 50)]
    limit: usize,
}

#[derive(Subcommand)]
enum ActivityCommand {
    /// What an agent's call sent and how it ended, secrets removed.
    Show {
        /// Event id from hoist activity.
        event: i64,
    },
}

pub(crate) async fn activity(context: &Context, args: ActivityArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    if let Some(ActivityCommand::Show { event }) = args.command {
        let found = client.get_json(&format!("/events/{event}/detail")).await?;
        return ui::show(&found, |found| {
            if found["detail"].is_null() {
                ui::note("That event keeps no detail.");
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&found["detail"]).unwrap_or_default()
                );
            }
        });
    }
    let agents = client.get_json("/agents").await.unwrap_or(Value::Null);
    let agent_list = agents["items"].as_array().cloned().unwrap_or_default();
    let mut pairs: Vec<(String, String)> = Vec::new();
    if let Some(project) = &args.project {
        let project = context::project(context, &client, Some(project), "").await?;
        pairs.push(("project_id".into(), project.id));
    }
    if let Some(wanted) = &args.agent {
        let id = agent_list
            .iter()
            .find(|agent| {
                agent["id"] == wanted.as_str()
                    || agent["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case(wanted))
            })
            .map_or_else(|| wanted.clone(), |agent| ui::text(agent, "id"));
        pairs.push(("actor".into(), id));
        pairs.push(("kind".into(), "agent.tool".into()));
    } else if args.agents {
        pairs.push(("kind".into(), "agent.tool".into()));
    }
    let projects = context::projects(&client).await.unwrap_or_default();
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    while items.len() < args.limit {
        let mut query: Vec<(&str, &str)> = pairs
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        if let Some(cursor) = &cursor {
            query.push(("cursor", cursor));
        }
        let page = client
            .get_json(&format!("/activity?{}", context::query(&query)))
            .await?;
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    items.truncate(args.limit);
    let who = |actor: &Value| {
        agent_list
            .iter()
            .find(|agent| agent["id"] == *actor)
            .map_or_else(
                || {
                    if actor == "person" {
                        "you".into()
                    } else {
                        ui::text(actor, "")
                    }
                },
                |agent| ui::text(agent, "name"),
            )
    };
    let project_name = |id: &Value| {
        projects
            .iter()
            .find(|project| project.id == id.as_str().unwrap_or_default())
            .map_or_else(|| ui::text(id, ""), |project| ui::clean(&project.name))
    };
    ui::list(
        &Value::Array(items),
        "Nothing has happened yet.",
        |item| {
            vec![
                ui::when(item, "at"),
                who(&item["actor"]),
                ui::text(item, "summary"),
                project_name(&item["project_id"]),
                ui::text(item, "id"),
            ]
        },
        &["WHEN", "WHO", "WHAT", "PROJECT", "EVENT"],
    )
}

// hoist billing

#[derive(Args)]
pub(crate) struct BillingArgs {}

fn money(value: &Value) -> Option<String> {
    let cents = value["cents"].as_i64()?;
    let currency = value["currency"].as_str().unwrap_or("USD");
    let symbol = if currency == "USD" { "$" } else { "" };
    let suffix = if currency == "USD" {
        String::new()
    } else {
        format!(" {currency}")
    };
    Some(format!(
        "{symbol}{}.{:02}{suffix}",
        cents / 100,
        (cents % 100).abs()
    ))
}

pub(crate) async fn billing(context: &Context, _args: BillingArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let found = client.get_json("/billing").await?;
    ui::show(&found, |found| {
        let rows = found.as_array().cloned().unwrap_or_default();
        if rows.is_empty() {
            ui::note("No connected account reports billing.");
            return;
        }
        let mut table = ui::Table::new(&[
            "ACCOUNT",
            "PROVIDER",
            "BILLED SO FAR",
            "PERIOD ENDS",
            "LIMIT",
        ]);
        let mut totals = std::collections::BTreeMap::<String, i64>::new();
        for row in &rows {
            let bill = &row["bill"];
            if bill.is_null() {
                continue;
            }
            if let (Some(cents), Some(currency)) = (
                bill["billed"]["cents"].as_i64(),
                bill["billed"]["currency"].as_str(),
            ) {
                *totals.entry(currency.to_owned()).or_default() += cents;
            }
            let limit = money(&bill["hard_limit"])
                .or_else(|| money(&bill["soft_limit"]))
                .unwrap_or_else(|| "-".into());
            table.row(vec![
                ui::text(row, "label"),
                ui::text(row, "provider"),
                money(&bill["billed"]).unwrap_or_else(|| "-".into()),
                bill["period_end"]
                    .as_str()
                    .map_or_else(|| "-".into(), |end| end.chars().take(10).collect()),
                if bill["over_limit"] == true {
                    format!("{limit} (over)")
                } else {
                    limit
                },
            ]);
        }
        if !table.is_empty() {
            table.print();
            for (currency, cents) in totals {
                println!(
                    "\nTotal reported: {}",
                    money(&json!({"cents": cents, "currency": currency})).unwrap_or_default()
                );
            }
        }
        for row in &rows {
            let usage = row["plan_usage"].as_array().cloned().unwrap_or_default();
            if usage.is_empty() && row["note"].is_null() {
                continue;
            }
            println!(
                "\n{} ({})",
                ui::text(row, "label"),
                ui::text(row, "provider")
            );
            for line in usage {
                let used = line["used"].as_f64().unwrap_or(0.0);
                let limit = line["limit"]
                    .as_f64()
                    .map(|limit| format!(" of {limit}"))
                    .unwrap_or_default();
                println!(
                    "  {}  {used}{limit} {}",
                    ui::text(&line, "label"),
                    ui::text(&line, "unit")
                );
            }
            if let Some(note) = row["note"].as_str() {
                println!("  {}", ui::clean(note));
            }
        }
        ui::note("\nThese are the numbers each provider reports. Hoista doesn't estimate.");
    })
}

// hoist usage

#[derive(Args)]
pub(crate) struct UsageArgs {
    /// Project name or id. Default: the project linked to this folder.
    project: Option<String>,
    /// From this day, as YYYY-MM-DD. Default: 30 days ago.
    #[arg(long, value_name = "DATE")]
    since: Option<String>,
}

pub(crate) async fn usage(context: &Context, args: UsageArgs) -> anyhow::Result<()> {
    let client = context.client().await?;
    let project = context::project(
        context,
        &client,
        args.project.as_deref(),
        "Name the project: hoist usage PROJECT.",
    )
    .await?;
    let since = match args.since {
        Some(since) => {
            if chrono::NaiveDate::parse_from_str(&since, "%Y-%m-%d").is_err() {
                return ui::usage("Write --since as YYYY-MM-DD.");
            }
            since
        }
        None => (chrono::Utc::now() - chrono::Duration::days(DEFAULT_USAGE_DAYS))
            .format("%Y-%m-%d")
            .to_string(),
    };
    let resources = context::resources(&client, &project).await?;
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..100 {
        let mut query = vec![("since", since.as_str())];
        if let Some(cursor) = &cursor {
            query.push(("cursor", cursor));
        }
        let page = client
            .get_json(&format!(
                "{}?{}",
                context::path(&["projects", &project.id, "usage"]),
                context::query(&query)
            ))
            .await?;
        items.extend(page["items"].as_array().cloned().unwrap_or_default());
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    let name = |id: &Value| {
        resources
            .iter()
            .find(|resource| resource["id"] == *id)
            .map_or_else(|| ui::text(id, ""), |resource| ui::text(resource, "name"))
    };
    let count = |value: &Value| {
        value
            .as_i64()
            .map_or_else(|| "-".into(), |value| value.to_string())
    };
    let size = |value: &Value| {
        value
            .as_f64()
            .map_or_else(|| "-".into(), |value| number(value, "bytes"))
    };
    ui::list(
        &Value::Array(items),
        "The providers reported no usage for that range.",
        |item| {
            vec![
                ui::text(item, "day"),
                name(&item["resource_id"]),
                count(&item["requests"]),
                count(&item["errors"]),
                item["compute_seconds"]
                    .as_f64()
                    .map_or_else(|| "-".into(), |value| number(value, "seconds")),
                size(&item["storage_bytes"]),
                size(&item["egress_bytes"]),
            ]
        },
        &[
            "DAY", "RESOURCE", "REQUESTS", "ERRORS", "COMPUTE", "STORAGE", "EGRESS",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headlines_follow_the_chart_and_money_reads_right() {
        let points = vec![json!({"value": 1.0}), json!({"value": 3.0})];
        assert_eq!(headline(&points, "total"), Some(4.0));
        assert_eq!(headline(&points, "average"), Some(2.0));
        assert_eq!(headline(&points, "latest"), Some(3.0));
        assert_eq!(headline(&[], "total"), None);
        assert_eq!(
            money(&json!({"cents": 1205, "currency": "USD"})).unwrap(),
            "$12.05"
        );
        assert_eq!(
            money(&json!({"cents": 7, "currency": "EUR"})).unwrap(),
            "0.07 EUR"
        );
        assert_eq!(number(1_500_000.0, "bytes"), "1.5 MB");
    }
}
