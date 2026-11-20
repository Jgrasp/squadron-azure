use crate::api;
use crate::reads::{Repo, me_id};
use serde_json::{Map, Value, json};
use squadron_extension_api::forge::{
    Draft, MergeArgs, PostArgs, Posted, ReplyArgs, ResolveArgs, UpdateArgs,
};

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

pub fn completion(found: &Value, args: &MergeArgs) -> Result<Value, String> {
    let mut body =
        json!({ "status": "completed", "lastMergeSourceCommit": found["lastMergeSourceCommit"] });
    if args.method.is_none() && args.delete_branch.is_none() {
        return Ok(body);
    }
    let mut options = found["completionOptions"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(method) = &args.method {
        let strategy = match method.as_str() {
            "merge" => "noFastForward",
            "squash" => "squash",
            "rebase" => "rebase",
            other => return Err(format!("Azure DevOps cannot merge with {other}.")),
        };
        options.insert("mergeStrategy".into(), json!(strategy));
    }
    if let Some(delete) = args.delete_branch {
        options.insert("deleteSourceBranch".into(), json!(delete));
    }
    body["completionOptions"] = Value::Object(options);
    Ok(body)
}

pub fn merge(repo: &Repo, args: &MergeArgs) -> Result<(), String> {
    let path = repo.pull_api(args.number);
    let found = repo.get(&path, &[])?;
    send(repo, "PATCH", &path, Some(completion(&found, args)?)).map(|_| ())
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
    fn completes_with_the_method_and_branch_asked() {
        let found = json!({
            "lastMergeSourceCommit": { "commitId": "3aae" },
            "completionOptions": { "mergeCommitMessage": "Merged PR 5", "deleteSourceBranch": true }
        });
        let asked = |method: Option<&str>, delete_branch| MergeArgs {
            number: 5,
            method: method.map(str::to_string),
            delete_branch,
        };
        assert_eq!(
            completion(&found, &asked(None, None)),
            Ok(json!({ "status": "completed", "lastMergeSourceCommit": { "commitId": "3aae" } }))
        );
        assert_eq!(
            completion(&found, &asked(Some("rebase"), Some(false))).unwrap()["completionOptions"],
            json!({ "mergeCommitMessage": "Merged PR 5", "deleteSourceBranch": false, "mergeStrategy": "rebase" })
        );
        let squash = completion(&json!({}), &asked(Some("squash"), None)).unwrap();
        assert_eq!(
            squash["completionOptions"],
            json!({ "mergeStrategy": "squash" })
        );
        assert_eq!(
            completion(&found, &asked(Some("merge"), None)).unwrap()["completionOptions"]["mergeStrategy"],
            "noFastForward"
        );
        assert!(completion(&found, &asked(Some("fast-forward"), None)).is_err());
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
