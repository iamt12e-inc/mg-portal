# Mugonat Portal

A small Linux desktop app that watches selected NetworkManager Wi-Fi connections and signs in to a captive portal after the session expires.

The first profile targets these Wi-Fi names:

- `Mugonat Systems 5.0`
- `Mugonat Systems 2.4`

The login form is discovered on the connected network. The app only submits credentials after you trust the portal origin. Credentials are stored in the desktop Secret Service keyring and are never written to the settings file or logs.

## Current limitations

- The app currently supports NetworkManager on Linux desktop sessions.
- It can submit ordinary HTML username/password forms with hidden fields and cookies. CAPTCHA, one-time codes, JavaScript-only flows, and unusual form requirements open the portal for manual sign-in.
- The actual Mugonat login page has not yet been inspected from the target Wi-Fi, so the first connection must be used to confirm the form and portal host.
- NetworkManager must have the saved Wi-Fi profiles configured to connect automatically. This app handles portal sign-in after NetworkManager connects.

## Development

Build dependencies include Rust, Cargo, GTK 4 development files, and a Secret Service provider such as GNOME Keyring or KWallet. The app uses NetworkManager to check the active Wi-Fi connection and the desktop keyring to store sign-in details.

```sh
cargo run
```

## Linux installers

The GitHub Actions workflow builds an AppImage, a Debian package, and an RPM on Blacksmith whenever `main` changes, when a pull request is opened, or when started manually. Those runs upload a `mugonat-portal-linux-installers` artifact.

For a versioned release, update the package version in `Cargo.toml` and push a matching version tag such as `v0.1.1`. The workflow builds the installers on Blacksmith and publishes them as assets on the GitHub Release for that tag, with generated release notes.

The AppImage bundles the application and its GTK libraries. Debian and RPM packages install the launcher, app icon, and desktop metadata in the standard system locations; they rely on the host's NetworkManager and desktop libraries.

## Start when you sign in

The **Start when you sign in** setting adds a standard desktop startup entry to your user account. Turn it off in the app to remove that entry.

## Existing usage extension

The separate `internet-usage-bar` GNOME extension reads `http://login.net/status` for hotspot usage. The app's **View hotspot usage** button opens that page, but it does not establish the router hardware or the portal's sign-in protocol. This app does not modify or depend on that extension.
