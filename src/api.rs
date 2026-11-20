use serde_json::{Value, json};
use squadron_extension_api::forge;
use squadron_extension_api::{Request, Response, host};

pub const SITE: &str = "https://dev.azure.com";

pub fn organization(given: &str) -> String {
    given
        .trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

pub fn chosen(accounts: &[(String, String)], org: &str) -> Option<String> {
    accounts
        .iter()
        .find(|(_, o)| organization(o) == org)
        .or(accounts.first())
        .map(|(id, _)| id.clone())
}

pub fn account(org: &str) -> Result<String, String> {
    let accounts: Vec<(String, String)> = host::accounts()
        .into_iter()
        .map(|a| {
            let org = a
                .fields
                .into_iter()
                .find(|(k, _)| k == "organization")
                .map(|(_, v)| v)
                .unwrap_or_default();
            (a.id, org)
        })
        .collect();
    chosen(&accounts, org).ok_or_else(|| {
        forge::no_account(json!({
            "en": "Add an Azure DevOps organization in Settings › Extensions › Azure DevOps.",
            "fr": "Ajoutez une organisation Azure DevOps dans Réglages › Extensions › Azure DevOps.",
        }))
    })
}

pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn url(base: &str, query: &[(&str, &str)]) -> String {
    let mut query: Vec<(&str, &str)> = query.to_vec();
    if !base.contains("api-version=") {
        query.push(("api-version", "7.1"));
    }
    let query: Vec<String> = query
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect();
    if query.is_empty() {
        return base.into();
    }
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}{}", query.join("&"))
}

pub fn request(method: &str, url: String, account: &str, body: Option<String>) -> Request {
    let mut headers = vec![
        (
            "Authorization".into(),
            format!("Basic {{{{base64:{account}.organization:{account}.token}}}}"),
        ),
        ("Accept".into(), "application/json".into()),
    ];
    if body.is_some() {
        headers.push(("Content-Type".into(), "application/json".into()));
    }
    Request {
        method: method.into(),
        url,
        headers,
        body,
    }
}

pub fn send(
    account: &str,
    method: &str,
    base: &str,
    query: &[(&str, &str)],
    body: Option<Value>,
) -> Result<Value, String> {
    let request = request(
        method,
        url(base, query),
        account,
        body.map(|b| b.to_string()),
    );
    answer(host::fetch(&request)?)
}

pub fn many(account: &str, urls: &[String]) -> Vec<Value> {
    let requests: Vec<Request> = urls
        .iter()
        .map(|u| request("GET", url(u, &[]), account, None))
        .collect();
    host::fetch_many(&requests)
        .into_iter()
        .map(|r| r.and_then(answer).unwrap_or_default())
        .collect()
}

pub fn connection_url(org: &str) -> String {
    format!("{SITE}/{org}/_apis/connectionData?api-version=7.1-preview")
}

pub fn user(found: Value) -> Result<Value, String> {
    let user = &found["authenticatedUser"];
    if user["id"].is_string() {
        Ok(user.clone())
    } else {
        Err(refused())
    }
}

pub fn connection(account: &str, org: &str) -> Result<Value, String> {
    user(send(account, "GET", &connection_url(org), &[], None)?)
}

pub fn refused() -> String {
    forge::no_account(json!({
        "en": "Azure DevOps refused the token: add a new one in Settings › Extensions › Azure DevOps.",
        "fr": "Azure DevOps a refusé le jeton : ajoutez-en un nouveau dans Réglages › Extensions › Azure DevOps.",
    }))
}

pub fn denied(error: &str) -> bool {
    error.contains("\"code\":\"account\"")
}

pub fn failure(status: u16, body: &Value, raw: &str) -> String {
    let reason = [&body["error"]["message"], &body["message"], &body["error"]]
        .into_iter()
        .find_map(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| raw.chars().take(200).collect());
    format!("Azure DevOps answered {status}: {reason}")
}

pub fn answer(response: Response) -> Result<Value, String> {
    if response.status == 401 || response.status == 203 {
        return Err(refused());
    }
    let body: Value = serde_json::from_str(&response.body).unwrap_or(Value::Null);
    if !(200..300).contains(&response.status) {
        return Err(failure(response.status, &body, &response.body));
    }
    if body.is_null() {
        return Err(refused());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, body: &str) -> Response {
        Response {
            status,
            headers: vec![],
            body: body.into(),
        }
    }

    #[test]
    fn reads_the_organization_as_typed() {
        assert_eq!(
            organization(" https://dev.azure.com/Fabrikam/ "),
            "fabrikam"
        );
        assert_eq!(organization("Fabrikam"), "fabrikam");
        assert_eq!(organization(""), "");
    }

    #[test]
    fn chooses_the_account_of_the_organization() {
        let all = vec![
            (
                "a1".to_string(),
                "https://dev.azure.com/Contoso".to_string(),
            ),
            ("a2".into(), "fabrikam".into()),
        ];
        assert_eq!(chosen(&all, "fabrikam").as_deref(), Some("a2"));
        assert_eq!(chosen(&all, "contoso").as_deref(), Some("a1"));
        assert_eq!(chosen(&all, "other").as_deref(), Some("a1"));
        assert_eq!(chosen(&[], "other"), None);
    }

    #[test]
    fn encodes_queries_azure_devops_reads() {
        assert_eq!(
            url(
                "https://dev.azure.com/o/p/_apis/git/repositories/r/pullrequests",
                &[
                    ("searchCriteria.sourceRefName", "refs/heads/feat/x y"),
                    ("$top", "1")
                ]
            ),
            "https://dev.azure.com/o/p/_apis/git/repositories/r/pullrequests?searchCriteria.sourceRefName=refs%2Fheads%2Ffeat%2Fx%20y&%24top=1&api-version=7.1"
        );
        assert_eq!(
            url(&connection_url("o"), &[]),
            "https://dev.azure.com/o/_apis/connectionData?api-version=7.1-preview"
        );
    }

    #[test]
    fn explains_why_azure_devops_refused() {
        let body = json!({ "message": "TF401180: The requested pull request was not found." });
        assert_eq!(
            failure(404, &body, ""),
            "Azure DevOps answered 404: TF401180: The requested pull request was not found."
        );
        assert_eq!(
            answer(response(500, "Oops")).unwrap_err(),
            "Azure DevOps answered 500: Oops"
        );
        for refusal in [
            response(401, ""),
            response(203, "<html>Sign in</html>"),
            response(200, "<html>Sign in</html>"),
        ] {
            assert!(denied(&answer(refusal).unwrap_err()));
        }
        assert_eq!(answer(response(200, r#"{"id":1}"#)), Ok(json!({ "id": 1 })));
        assert!(denied(
            &user(json!({ "authenticatedUser": {} })).unwrap_err()
        ));
        assert_eq!(
            user(json!({ "authenticatedUser": { "id": "u1" } })).unwrap()["id"],
            "u1"
        );
    }

    #[test]
    fn asks_squadron_for_basic_auth_with_the_account() {
        let sent = request("PATCH", url("https://x", &[]), "acc1", Some("{}".into()));
        assert_eq!(
            sent.headers[0],
            (
                "Authorization".to_string(),
                "Basic {{base64:acc1.organization:acc1.token}}".to_string()
            )
        );
        assert!(
            sent.headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "application/json")
        );
    }
}
