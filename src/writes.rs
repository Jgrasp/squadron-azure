use crate::api;
use crate::reads::{Repo, me_id};
use serde_json::{Map, Value, json};
use squadron_extension_api::forge::{Draft, PostArgs, Posted, ReplyArgs, ResolveArgs, UpdateArgs};

pub fn remark_id(to: &str) -> Result<(u64, u64), String> {
    to.split_once(':')
        .and_then(|(t, c)| Some((t.parse().ok()?, c.parse().ok()?)))
        .ok_or_else(|| format!("{to} is not an Azure DevOps comment."))
}

fn send(repo: &Repo, method: &str, url: &str, body: Option<Value>) -> Result<Value, String> {
    api::send(&repo.account, method, url, &[], body)
}

pub fn reply(repo: &Repo, args: &ReplyArgs) -> Result<(), String> {
    let (thread, comment) = remark_id(&args.to)?;
    send(
        repo,
        "POST",
        &format!("{}/threads/{thread}/comments", repo.pull_api(args.number)),
        Some(json!({ "content": args.body, "parentCommentId": comment, "commentType": 1 })),
    )
    .map(|_| ())
}

pub fn resolve(repo: &Repo, args: &ResolveArgs) -> Result<(), String> {
    let (thread, _) = remark_id(&args.to)?;
    let status = if args.resolved { "fixed" } else { "active" };
    send(
        repo,
        "PATCH",
        &format!("{}/threads/{thread}", repo.pull_api(args.number)),
        Some(json!({ "status": status })),
    )
    .map(|_| ())
}

pub fn thread(body: &str, draft: Option<&Draft>) -> Value {
    let mut thread = json!({
        "comments": [{ "parentCommentId": 0, "content": body, "commentType": 1 }],
        "status": "active",
    });
    if let Some(draft) = draft.filter(|d| !d.path.is_empty()) {
        let line = json!({ "line": draft.line, "offset": 1 });
        let side = if draft.removed { "left" } else { "right" };
        let context = &mut thread["threadContext"];
        context["filePath"] = json!(format!("/{}", draft.path.trim_start_matches('/')));
        context[format!("{side}FileStart")] = line.clone();
        context[format!("{side}FileEnd")] = line;
    }
    thread
}

pub fn vote(verdict: &str) -> Option<i64> {
    match verdict {
        "approve" => Some(10),
        "request_changes" => Some(-5),
        _ => None,
    }
}

pub fn plain(error: String) -> String {
    match serde_json::from_str::<Value>(&error) {
        Ok(found) => found["title"]["en"]
            .as_str()
            .or(found["title"].as_str())
            .map(str::to_string)
            .unwrap_or(error),
        Err(_) => error,
    }
}

pub fn post(repo: &Repo, args: &PostArgs) -> Result<Posted, String> {
    let path = repo.pull_api(args.number);
    let threads = format!("{path}/threads");
    for (done, draft) in args.drafts.iter().enumerate() {
        if let Err(error) = send(
            repo,
            "POST",
            &threads,
            Some(thread(&draft.body, Some(draft))),
        ) {
            if done == 0 && api::denied(&error) {
                return Err(error);
            }
            return Ok(Posted {
                done,
                said: false,
                error: Some(plain(error)),
            });
        }
    }
    let summary = args.summary.trim();
    let mut sent = if summary.is_empty() {
        Ok(Value::Null)
    } else {
        send(repo, "POST", &threads, Some(thread(summary, None)))
    };
    if let Err(error) = &sent {
        if args.drafts.is_empty() && api::denied(error) {
            return Err(error.clone());
        }
    }
    let said = sent.is_ok();
    if let (true, Some(vote)) = (said, vote(&args.verdict)) {
        sent = me_id(repo).and_then(|me| {
            send(
                repo,
                "PUT",
                &format!("{path}/reviewers/{me}"),
                Some(json!({ "vote": vote })),
            )
        });
    }
    Ok(Posted {
        done: args.drafts.len(),
        said,
        error: sent.err().map(plain),
    })
}

pub fn merge(repo: &Repo, number: u64) -> Result<(), String> {
    let path = repo.pull_api(number);
    let found = repo.get(&path, &[])?;
    send(
        repo,
        "PATCH",
        &path,
        Some(
            json!({ "status": "completed", "lastMergeSourceCommit": found["lastMergeSourceCommit"] }),
        ),
    )
    .map(|_| ())
}

pub fn changes(args: &UpdateArgs) -> Value {
    let mut changes = Map::new();
    if let Some(title) = &args.title {
        changes.insert("title".into(), json!(title));
    }
    if let Some(description) = &args.description {
        changes.insert("description".into(), json!(description));
    }
    Value::Object(changes)
}

pub fn update(repo: &Repo, args: &UpdateArgs) -> Result<(), String> {
    send(
        repo,
        "PATCH",
        &repo.pull_api(args.number),
        Some(changes(args)),
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(path: &str, line: u32, removed: bool, body: &str) -> Draft {
        Draft {
            path: path.into(),
            line,
            removed,
            body: body.into(),
        }
    }

    #[test]
    fn places_a_draft_on_its_side_of_the_diff() {
        let posted = thread("Why?", Some(&draft("src/a.cs", 4, true, "Why?")));
        assert_eq!(
            posted,
            json!({
                "comments": [{ "parentCommentId": 0, "content": "Why?", "commentType": 1 }],
                "status": "active",
                "threadContext": {
                    "filePath": "/src/a.cs",
                    "leftFileStart": { "line": 4, "offset": 1 },
                    "leftFileEnd": { "line": 4, "offset": 1 }
                }
            })
        );
        let added = thread("Rename", Some(&draft("/src/a.cs", 9, false, "Rename")));
        assert_eq!(added["threadContext"]["filePath"], "/src/a.cs");
        assert_eq!(added["threadContext"]["rightFileEnd"]["line"], 9);
        assert!(thread("LGTM", None)["threadContext"].is_null());
        assert!(
            thread("Add a test.", Some(&draft("", 0, false, "Add a test.")))["threadContext"]
                .is_null()
        );
    }

    #[test]
    fn gives_the_verdict_its_vote() {
        assert_eq!(vote("approve"), Some(10));
        assert_eq!(vote("request_changes"), Some(-5));
        assert_eq!(vote("comment"), None);
        assert_eq!(vote(""), None);
    }

    #[test]
    fn sends_only_what_changes() {
        let both = UpdateArgs {
            number: 3,
            title: Some("Export".into()),
            description: Some("Adds export".into()),
        };
        assert_eq!(
            changes(&both),
            json!({ "title": "Export", "description": "Adds export" })
        );
        let title = UpdateArgs {
            number: 3,
            title: Some("Export".into()),
            description: None,
        };
        assert_eq!(changes(&title), json!({ "title": "Export" }));
    }

    #[test]
    fn reads_the_thread_and_comment_of_a_remark() {
        assert_eq!(remark_id("11:2"), Ok((11, 2)));
        assert!(remark_id("720897").is_err());
        assert!(remark_id("11:x").is_err());
        assert_eq!(
            plain(api::refused()),
            "Azure DevOps refused the token: add a new one in Settings › Extensions › Azure DevOps."
        );
        assert_eq!(
            plain("Azure DevOps answered 400: no".into()),
            "Azure DevOps answered 400: no"
        );
    }
}
