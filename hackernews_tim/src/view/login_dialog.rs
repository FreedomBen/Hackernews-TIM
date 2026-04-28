use std::path::PathBuf;

use crate::prelude::*;

const USERNAME_ID: &str = "login_dialog_username";
const PASSWORD_ID: &str = "login_dialog_password";
const STATUS_ID: &str = "login_dialog_status";

/// Build a login dialog that collects a username/password, verifies them
/// against Hacker News by reusing the live [`HNClient`], and on success
/// persists the credentials using `storage` (and updates `auth_file` —
/// either as full credentials or as a keyring pointer file).
pub fn get_login_dialog(
    client: &'static client::HNClient,
    auth_file: PathBuf,
    storage: config::AuthStorage,
) -> impl View {
    let layout = LinearLayout::vertical()
        .child(TextView::new("Username:"))
        .child(EditView::new().with_name(USERNAME_ID).fixed_width(32))
        .child(DummyView)
        .child(TextView::new("Password:"))
        .child(
            EditView::new()
                .secret()
                .with_name(PASSWORD_ID)
                .fixed_width(32),
        )
        .child(DummyView)
        .child(TextView::new("").with_name(STATUS_ID));

    Dialog::around(layout)
        .title("Log in to Hacker News")
        .button("Cancel", |s| {
            s.pop_layer();
        })
        .button("Log in", move |s| {
            let username = s
                .call_on_name(USERNAME_ID, |v: &mut EditView| v.get_content())
                .map(|rc| rc.to_string())
                .unwrap_or_default();
            let password = s
                .call_on_name(PASSWORD_ID, |v: &mut EditView| v.get_content())
                .map(|rc| rc.to_string())
                .unwrap_or_default();

            if username.is_empty() || password.is_empty() {
                set_status(s, "Username and password are required.");
                return;
            }

            // `client.login` both verifies and establishes the session on the
            // live client, so we only write the file if it succeeds.
            match client.login(&username, &password) {
                Ok(()) => {
                    let auth = config::Auth {
                        username,
                        password,
                        session: client.current_session_cookie(),
                        storage,
                    };
                    match auth.write_to_file(&auth_file) {
                        Ok(()) => {
                            s.pop_layer();
                            let saved_to = match storage {
                                config::AuthStorage::File => {
                                    format!("Credentials saved to {}.", auth_file.display())
                                }
                                config::AuthStorage::Keyring => format!(
                                    "Credentials stored in OS keyring (service \"{}\"); pointer file at {}.",
                                    config::KEYRING_SERVICE,
                                    auth_file.display()
                                ),
                            };
                            s.add_layer(
                                Dialog::info(format!("Logged in as {}. {saved_to}", auth.username))
                                    .title("Login successful"),
                            );
                        }
                        Err(err) => {
                            set_status(
                                s,
                                &format!("Logged in, but failed to save credentials: {err}"),
                            );
                        }
                    }
                }
                Err(err) => {
                    set_status(s, &format!("Login failed: {err}"));
                }
            }
        })
        .max_width(60)
}

fn set_status(s: &mut Cursive, msg: &str) {
    let msg = msg.to_string();
    s.call_on_name(STATUS_ID, move |v: &mut TextView| v.set_content(msg));
}
