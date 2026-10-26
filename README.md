# Mugonat Portal

Designed for 64-bit Linux desktops from Ubuntu 20.04 and Fedora 32 onward. Requires NetworkManager and a desktop keyring. Releases include an AppImage, Debian package, and RPM.

Supports `Mugonat Systems 5.0` and `Mugonat Systems 2.4`. Sign-in details are stored in your desktop keyring. Forms the app cannot handle open in your browser for manual sign-in.

## Install

GitHub Releases provide an AppImage, Debian package, and RPM. In Settings, turn on **Start when you sign in** to keep the app running in the background.

## Publish a release

The app version in `Cargo.toml` changes when you choose to publish a new app release. Push a matching `v*` tag to build and publish the installers. Each build and retry gets an automatic number in the package revisions and AppImage filename.
