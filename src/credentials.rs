use anyhow::{Context, Result, anyhow};
use keyring::Entry;

const SERVICE: &str = "mugonat-portal";
const USERNAME_KEY: &str = "portal-username";
const PASSWORD_KEY: &str = "portal-password";

pub fn store(username: &str, password: &str) -> Result<()> {
    Entry::new(SERVICE, USERNAME_KEY)
        .context("Could not connect to the desktop keyring")?
        .set_password(username)
        .context("Could not save the username in the desktop keyring")?;
    Entry::new(SERVICE, PASSWORD_KEY)
        .context("Could not connect to the desktop keyring")?
        .set_password(password)
        .context("Could not save the password in the desktop keyring")?;
    Ok(())
}

pub fn load() -> Result<Option<(String, String)>> {
    let username = read_entry(USERNAME_KEY)?;
    let password = read_entry(PASSWORD_KEY)?;
    match (username, password) {
        (Some(username), Some(password)) if !username.is_empty() && !password.is_empty() => {
            Ok(Some((username, password)))
        }
        (None, None) => Ok(None),
        _ => Err(anyhow!(
            "The saved portal sign-in is incomplete. Save it again in the app."
        )),
    }
}

pub fn saved_username() -> Option<String> {
    read_entry(USERNAME_KEY).ok().flatten()
}

fn read_entry(account: &str) -> Result<Option<String>> {
    let entry = Entry::new(SERVICE, account).context("Could not connect to the desktop keyring")?;
    match entry.get_password() {
        Ok(password) => Ok(Some(password)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error).context("Could not read from the desktop keyring"),
    }
}
