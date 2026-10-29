use anyhow::{anyhow, Context, Result};
use md5::{Digest, Md5};
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use scraper::{Html, Selector};
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

const PROBE_URL: &str = "http://connectivitycheck.gstatic.com/generate_204";
const PROBE_HOST: &str = "connectivitycheck.gstatic.com";
const MAX_PAGE_BYTES: u64 = 256 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
pub enum Outcome {
    Online,
    NeedsCredentials {
        page_url: String,
    },
    SignedIn,
    Manual {
        page_url: Option<String>,
        reason: String,
        submission_made: bool,
    },
}

pub fn is_probe_url(raw_url: &str) -> bool {
    Url::parse(raw_url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.eq_ignore_ascii_case(PROBE_HOST))
        })
        .unwrap_or(false)
}

#[derive(Debug)]
struct LoginForm {
    action: Url,
    fields: Vec<(String, String)>,
    chap: Option<ChapChallenge>,
}

#[derive(Debug)]
struct ChapChallenge {
    id: Vec<u8>,
    challenge: Vec<u8>,
}

#[derive(Debug)]
enum PageResult {
    Online,
    LoginPage { page_url: Url, form: LoginForm },
    OtherPage { page_url: Url },
}

pub fn check_and_login(
    credentials: Option<(&str, &str)>,
    allow_submission: bool,
) -> Result<Outcome> {
    let cookies = Arc::new(reqwest::cookie::Jar::default());
    let client = Client::builder()
        .cookie_provider(cookies.clone())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(Policy::limited(8))
        .build()
        .context("Could not prepare the portal connection")?;
    let submission_client = Client::builder()
        .cookie_provider(cookies)
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(Policy::none())
        .build()
        .context("Could not prepare the sign-in request")?;

    let page = match fetch_page(&client)? {
        PageResult::Online => return Ok(Outcome::Online),
        PageResult::LoginPage { page_url, form } => (page_url, form),
        PageResult::OtherPage { page_url } => {
            return Ok(Outcome::Manual {
                page_url: Some(page_url.to_string()),
                reason: "This sign-in page needs a step the app can't complete. Open the page to continue.".to_owned(),
                submission_made: false,
            });
        }
    };

    let (page_url, form) = page;
    let Some((username, password)) = credentials else {
        return Ok(Outcome::NeedsCredentials {
            page_url: page_url.to_string(),
        });
    };

    if !allow_submission {
        return Ok(Outcome::Manual {
            page_url: Some(page_url.to_string()),
            reason: "Automatic sign-in paused after a failed attempt. Open the sign-in page and try again.".to_owned(),
            submission_made: false,
        });
    }

    // Only the two credential fields are injected after the form and its current
    // hidden values have been inspected. None of this data is logged or persisted here.
    let fields = replace_credentials(form.fields, username, password, form.chap.as_ref());
    if let Err(error) = submission_client.post(form.action).form(&fields).send() {
        eprintln!("Sign-in submission failed: {error:#}");
        return Ok(Outcome::Manual {
            page_url: Some(page_url.to_string()),
            reason: "Sign-in didn't finish. Open the sign-in page and try again.".to_owned(),
            submission_made: true,
        });
    }

    match fetch_page(&client) {
        Ok(PageResult::Online) => Ok(Outcome::SignedIn),
        Ok(PageResult::LoginPage { page_url, .. }) => Ok(Outcome::Manual {
            page_url: Some(page_url.to_string()),
            reason: "The portal still requires sign-in. Check the username and password."
                .to_owned(),
            submission_made: true,
        }),
        Ok(PageResult::OtherPage { page_url }) => Ok(Outcome::Manual {
            page_url: Some(page_url.to_string()),
            reason: "The app could not confirm that sign-in succeeded.".to_owned(),
            submission_made: true,
        }),
        Err(error) => {
            eprintln!("Could not confirm portal sign-in: {error:#}");
            Ok(Outcome::Manual {
                page_url: Some(page_url.to_string()),
                reason: "Couldn't confirm that sign-in worked. Open the sign-in page and check your connection.".to_owned(),
                submission_made: true,
            })
        }
    }
}

fn replace_credentials(
    fields: Vec<(String, String)>,
    username: &str,
    password: &str,
    chap: Option<&ChapChallenge>,
) -> Vec<(String, String)> {
    // The parser stores the field names in these internal marker values. Replace
    // them only at the final point where the HTTP request is assembled.
    fields
        .into_iter()
        .map(|(name, value)| match value.as_str() {
            "\u{0}mugonat-username" => (name, username.to_owned()),
            "\u{0}mugonat-password" => (name, encoded_password(password, chap)),
            _ => (name, value),
        })
        .collect()
}

fn encoded_password(password: &str, chap: Option<&ChapChallenge>) -> String {
    let Some(chap) = chap else {
        return password.to_owned();
    };

    // MikroTik's HTTP-CHAP page computes MD5(chap-id + password + challenge)
    // in JavaScript before submitting. Match its 8-bit string handling.
    let mut input = chap.id.clone();
    input.extend(password.chars().map(|character| character as u8));
    input.extend_from_slice(&chap.challenge);
    format!("{:x}", Md5::digest(input))
}

fn fetch_page(client: &Client) -> Result<PageResult> {
    let response = client
        .get(PROBE_URL)
        .send()
        .context("Could not check the network connection")?;
    let status = response.status();
    let page_url = response.url().clone();
    if status.as_u16() == 204 && page_url.as_str() == PROBE_URL {
        return Ok(PageResult::Online);
    }

    let html = read_limited(response)?;
    match parse_login_form(&html, &page_url) {
        Ok(Some(form)) => Ok(PageResult::LoginPage { page_url, form }),
        Ok(None) => Ok(PageResult::OtherPage { page_url }),
        Err(_) => Ok(PageResult::OtherPage { page_url }),
    }
}

fn read_limited(response: Response) -> Result<String> {
    let mut bytes = Vec::new();
    response
        .take(MAX_PAGE_BYTES)
        .read_to_end(&mut bytes)
        .context("Could not read the portal page")?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_login_form(html: &str, page_url: &Url) -> Result<Option<LoginForm>> {
    let document = Html::parse_document(html);
    let form_selector = Selector::parse("form").map_err(|_| anyhow!("Invalid form selector"))?;
    let input_selector = Selector::parse("input").map_err(|_| anyhow!("Invalid input selector"))?;
    let button_selector =
        Selector::parse("button").map_err(|_| anyhow!("Invalid button selector"))?;
    let chap = mikrotik_challenge(html);
    if html.to_ascii_lowercase().contains("hexmd5") && chap.is_none() {
        // Never fall back to sending a plain password when the page indicates
        // that it expects a CHAP response we could not read.
        return Ok(None);
    }

    for form in document.select(&form_selector) {
        let mut password_names = Vec::new();
        let mut text_candidates = Vec::new();
        let mut preferred_usernames = Vec::new();
        let mut fields = Vec::new();
        let mut unsupported = false;

        for input in form.select(&input_selector) {
            let value = input.value();
            if value.attr("disabled").is_some() {
                continue;
            }
            let Some(name) = value.attr("name").filter(|name| !name.trim().is_empty()) else {
                continue;
            };
            let kind = value.attr("type").unwrap_or("text").to_ascii_lowercase();
            let initial_value = value.attr("value").unwrap_or("").to_owned();
            let label = format!(
                "{} {} {}",
                name,
                value.attr("id").unwrap_or(""),
                value.attr("autocomplete").unwrap_or("")
            )
            .to_ascii_lowercase();

            match kind.as_str() {
                "password" => {
                    password_names.push(name.to_owned());
                    fields.push((name.to_owned(), "\u{0}mugonat-password".to_owned()));
                }
                "text" | "email" => {
                    text_candidates.push(name.to_owned());
                    if ["user", "login", "email", "account"]
                        .iter()
                        .any(|part| label.contains(part))
                    {
                        preferred_usernames.push(name.to_owned());
                    }
                    fields.push((name.to_owned(), initial_value));
                }
                "hidden" => fields.push((name.to_owned(), initial_value)),
                "submit" => fields.push((name.to_owned(), initial_value)),
                "checkbox" | "radio" => {
                    if value.attr("required").is_some() && value.attr("checked").is_none() {
                        unsupported = true;
                    } else if value.attr("checked").is_some() {
                        fields.push((name.to_owned(), initial_value));
                    }
                }
                "button" | "reset" | "file" | "image" => {}
                _ => unsupported = true,
            }
        }

        if unsupported || password_names.len() != 1 {
            continue;
        }

        let username_name = if preferred_usernames.len() == 1 {
            preferred_usernames.first()
        } else if preferred_usernames.is_empty() && text_candidates.len() == 1 {
            text_candidates.first()
        } else {
            None
        };
        let Some(username_name) = username_name else {
            continue;
        };

        let Some(username_field) = fields.iter_mut().find(|(name, _)| name == username_name) else {
            continue;
        };
        username_field.1 = "\u{0}mugonat-username".to_owned();

        let action_text = form.value().attr("action").unwrap_or(page_url.as_str());
        let action = page_url
            .join(action_text)
            .context("The portal form has an invalid sign-in address")?;
        if !matches!(action.scheme(), "http" | "https")
            || action.host_str().is_none()
            || !action.username().is_empty()
            || action.password().is_some()
        {
            continue;
        }
        let method = form
            .value()
            .attr("method")
            .unwrap_or("get")
            .trim()
            .to_ascii_lowercase();
        // Never put credentials in a URL. GET-based forms are left to the browser.
        if method != "post" {
            continue;
        }
        if let Some(enctype) = form.value().attr("enctype") {
            let enctype = enctype.split(';').next().unwrap_or_default().trim();
            if !enctype.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
                continue;
            }
        }

        let unsupported_control_selector = Selector::parse("select[name], textarea[name]")
            .map_err(|_| anyhow!("Invalid form control selector"))?;
        if form.select(&unsupported_control_selector).any(|control| {
            control.value().attr("disabled").is_none()
                && control
                    .value()
                    .attr("name")
                    .is_some_and(|name| !name.trim().is_empty())
        }) {
            continue;
        }

        // Preserve named submit buttons, which some basic portal forms require.
        if let Some(button) = form.select(&button_selector).find(|button| {
            let button_type = button.value().attr("type").unwrap_or("submit");
            button_type.eq_ignore_ascii_case("submit")
        }) {
            if let Some(name) = button.value().attr("name") {
                fields.push((
                    name.to_owned(),
                    button.value().attr("value").unwrap_or("").to_owned(),
                ));
            }
        }

        return Ok(Some(LoginForm {
            action,
            fields,
            chap,
        }));
    }

    Ok(None)
}

fn mikrotik_challenge(html: &str) -> Option<ChapChallenge> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("hexmd5")?;
    let open = html[start..].find('(')? + start;
    let close = js_call_end(html, open)?;
    let literals = js_string_literals(&html[open + 1..close]);
    if literals.len() < 2 {
        return None;
    }
    let id = literals.first()?.clone();
    let challenge = literals.last()?.clone();
    if id.is_empty() || challenge.is_empty() {
        return None;
    }
    Some(ChapChallenge { id, challenge })
}

fn js_call_end(source: &str, open: usize) -> Option<usize> {
    let mut quote = None;
    let mut escaped = false;
    let mut depth = 0usize;
    for (offset, character) in source[open..].char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }

        match character {
            '\'' | '"' | '`' => quote = Some(character),
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn js_string_literals(source: &str) -> Vec<Vec<u8>> {
    let characters: Vec<char> = source.chars().collect();
    let mut literals = Vec::new();
    let mut index = 0usize;
    while index < characters.len() {
        let quote = characters[index];
        if quote != '\'' && quote != '"' {
            index += 1;
            continue;
        }
        index += 1;
        let mut value = Vec::new();
        while index < characters.len() && characters[index] != quote {
            let character = characters[index];
            index += 1;
            if character != '\\' {
                value.push(character as u8);
                continue;
            }
            if index >= characters.len() {
                break;
            }
            let escaped = characters[index];
            index += 1;
            match escaped {
                'x' => {
                    if let Some(byte) = parse_hex_escape(&characters, &mut index, 2) {
                        value.push(byte);
                    }
                }
                'u' => {
                    if let Some(byte) = parse_hex_escape(&characters, &mut index, 4) {
                        value.push(byte);
                    }
                }
                '0'..='7' => {
                    let mut digits = String::from(escaped);
                    while digits.len() < 3
                        && index < characters.len()
                        && ('0'..='7').contains(&characters[index])
                    {
                        digits.push(characters[index]);
                        index += 1;
                    }
                    if let Ok(parsed) = u16::from_str_radix(&digits, 8) {
                        value.push(parsed as u8);
                    }
                }
                '\n' => {}
                '\r' => {
                    if characters.get(index) == Some(&'\n') {
                        index += 1;
                    }
                }
                'n' => value.push(b'\n'),
                'r' => value.push(b'\r'),
                't' => value.push(b'\t'),
                'b' => value.push(8),
                'f' => value.push(12),
                other => value.push(other as u8),
            }
        }
        if index < characters.len() {
            index += 1;
        }
        literals.push(value);
    }
    literals
}

fn parse_hex_escape(characters: &[char], index: &mut usize, digits: usize) -> Option<u8> {
    let end = (*index).checked_add(digits)?;
    let text: String = characters.get(*index..end)?.iter().collect();
    *index = end;
    u32::from_str_radix(&text, 16).ok().map(|value| value as u8)
}
