use anyhow::Error;
use const_format::concatcp;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use handlebars::{Handlebars, TemplateError};
use nix::unistd::Uid;

use proxmox_lang::try_block;
use proxmox_notify::context::pbs::PBS_CONTEXT;
use proxmox_schema::ApiType;
use proxmox_sys::email::sendmail;
use proxmox_sys::fs::{create_path, CreateOptions};

use pbs_api_types::{
    APTUpdateInfo, DataStoreConfig, DatastoreNotify, GarbageCollectionStatus, NotificationMode,
    Notify, SyncJobConfig, TapeBackupJobSetup, User, Userid, VerificationJobConfig,
};
use proxmox_notify::endpoints::sendmail::{SendmailConfig, SendmailEndpoint};
use proxmox_notify::{Endpoint, Notification, Severity};

const SPOOL_DIR: &str = concatcp!(pbs_buildcfg::PROXMOX_BACKUP_STATE_DIR, "/notifications");

const VERIFY_OK_TEMPLATE: &str = r###"

Job ID:    {{job.id}}
Datastore: {{job.store}}

Verification successful.


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#DataStore-{{job.store}}>

"###;

const VERIFY_ERR_TEMPLATE: &str = r###"

Job ID:    {{job.id}}
Datastore: {{job.store}}

Verification failed on these snapshots/groups:

{{#each errors}}
  {{this~}}
{{/each}}


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:tasks>

"###;

const SYNC_OK_TEMPLATE: &str = r###"

Job ID:             {{job.id}}
Datastore:          {{job.store}}
{{#if job.remote~}}
Remote:             {{job.remote}}
Remote Store:       {{job.remote-store}}
{{else~}}
Local Source Store: {{job.remote-store}}
{{/if}}
Synchronization successful.


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#DataStore-{{job.store}}>

"###;

const SYNC_ERR_TEMPLATE: &str = r###"

Job ID:             {{job.id}}
Datastore:          {{job.store}}
{{#if job.remote~}}
Remote:             {{job.remote}}
Remote Store:       {{job.remote-store}}
{{else~}}
Local Source Store: {{job.remote-store}}
{{/if}}
Synchronization failed: {{error}}


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:tasks>

"###;

const PACKAGE_UPDATES_TEMPLATE: &str = r###"
Proxmox Backup Server has the following updates available:
{{#each updates }}
  {{Package}}: {{OldVersion}} -> {{Version~}}
{{/each }}

To upgrade visit the web interface:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:updates>

"###;

const TAPE_BACKUP_OK_TEMPLATE: &str = r###"

{{#if id ~}}
Job ID:     {{id}}
{{/if~}}
Datastore:  {{job.store}}
Tape Pool:  {{job.pool}}
Tape Drive: {{job.drive}}

{{#if snapshot-list ~}}
Snapshots included:

{{#each snapshot-list~}}
{{this}}
{{/each~}}
{{/if}}
Duration: {{duration}}
{{#if used-tapes }}
Used Tapes:
{{#each used-tapes~}}
{{this}}
{{/each~}}
{{/if}}
Tape Backup successful.


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#DataStore-{{job.store}}>

"###;

const TAPE_BACKUP_ERR_TEMPLATE: &str = r###"

{{#if id ~}}
Job ID:     {{id}}
{{/if~}}
Datastore:  {{job.store}}
Tape Pool:  {{job.pool}}
Tape Drive: {{job.drive}}

{{#if snapshot-list ~}}
Snapshots included:

{{#each snapshot-list~}}
{{this}}
{{/each~}}
{{/if}}
{{#if used-tapes }}
Used Tapes:
{{#each used-tapes~}}
{{this}}
{{/each~}}
{{/if}}
Tape Backup failed: {{error}}


Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsServerAdministration:tasks>

"###;

const ACME_CERTIFICATE_ERR_RENEWAL: &str = r###"

Proxmox Backup Server was not able to renew a TLS certificate.

Error: {{error}}

Please visit the web interface for further details:

<https://{{fqdn}}:{{port}}/#pbsCertificateConfiguration>

"###;

lazy_static::lazy_static! {

    static ref HANDLEBARS: Handlebars<'static> = {
        let mut hb = Handlebars::new();
        let result: Result<(), TemplateError> = try_block!({

            hb.set_strict_mode(true);
            hb.register_escape_fn(handlebars::no_escape);

            hb.register_template_string("verify_ok_template", VERIFY_OK_TEMPLATE)?;
            hb.register_template_string("verify_err_template", VERIFY_ERR_TEMPLATE)?;

            hb.register_template_string("sync_ok_template", SYNC_OK_TEMPLATE)?;
            hb.register_template_string("sync_err_template", SYNC_ERR_TEMPLATE)?;

            hb.register_template_string("tape_backup_ok_template", TAPE_BACKUP_OK_TEMPLATE)?;
            hb.register_template_string("tape_backup_err_template", TAPE_BACKUP_ERR_TEMPLATE)?;

            hb.register_template_string("package_update_template", PACKAGE_UPDATES_TEMPLATE)?;

            hb.register_template_string("certificate_renewal_err_template", ACME_CERTIFICATE_ERR_RENEWAL)?;

            Ok(())
        });

        if let Err(err) = result {
            eprintln!("error during template registration: {err}");
        }

        hb
    };
}

/// Initialize the notification system by setting context in proxmox_notify
pub fn init() -> Result<(), Error> {
    proxmox_notify::context::set_context(&PBS_CONTEXT);
    Ok(())
}

/// Create the directory which will be used to temporarily store notifications
/// which were sent from an unprivileged process.
pub fn create_spool_dir() -> Result<(), Error> {
    let backup_user = pbs_config::backup_user()?;
    let opts = CreateOptions::new()
        .owner(backup_user.uid)
        .group(backup_user.gid);

    create_path(SPOOL_DIR, None, Some(opts))?;
    Ok(())
}

async fn send_queued_notifications() -> Result<(), Error> {
    let mut read_dir = tokio::fs::read_dir(SPOOL_DIR).await?;

    let mut notifications = Vec::new();

    while let Some(entry) = read_dir.next_entry().await? {
        let path = entry.path();

        if let Some(ext) = path.extension() {
            if ext == "json" {
                let p = path.clone();

                let bytes = tokio::fs::read(p).await?;
                let notification: Notification = serde_json::from_slice(&bytes)?;
                notifications.push(notification);

                // Currently, there is no retry-mechanism in case of failure...
                // For retries, we'd have to keep track of which targets succeeded/failed
                // to send, so we do not retry notifying a target which succeeded before.
                tokio::fs::remove_file(path).await?;
            }
        }
    }

    // Make sure that we send the oldest notification first
    notifications.sort_unstable_by_key(|n| n.timestamp());

    let res = tokio::task::spawn_blocking(move || {
        let config = pbs_config::notifications::config()?;
        for notification in notifications {
            if let Err(err) = proxmox_notify::api::common::send(&config, &notification) {
                log::error!("failed to send notification: {err}");
            }
        }

        Ok::<(), Error>(())
    })
    .await?;

    if let Err(e) = res {
        log::error!("could not read notification config: {e}");
    }

    Ok::<(), Error>(())
}

/// Worker task to periodically send any queued notifications.
pub async fn notification_worker() {
    loop {
        let delay_target = Instant::now() + Duration::from_secs(5);

        if let Err(err) = send_queued_notifications().await {
            log::error!("notification worker task error: {err}");
        }

        tokio::time::sleep_until(tokio::time::Instant::from_std(delay_target)).await;
    }
}

fn send_notification(notification: Notification) -> Result<(), Error> {
    if nix::unistd::ROOT == Uid::current() {
        let config = pbs_config::notifications::config()?;
        proxmox_notify::api::common::send(&config, &notification)?;
    } else {
        let ser = serde_json::to_vec(&notification)?;
        let path = Path::new(SPOOL_DIR).join(format!("{id}.json", id = notification.id()));

        let backup_user = pbs_config::backup_user()?;
        let opts = CreateOptions::new()
            .owner(backup_user.uid)
            .group(backup_user.gid);
        proxmox_sys::fs::replace_file(path, &ser, opts, true)?;
        log::info!("queued notification (id={id})", id = notification.id())
    }

    Ok(())
}

fn send_sendmail_legacy_notification(notification: Notification, email: &str) -> Result<(), Error> {
    let endpoint = SendmailEndpoint {
        config: SendmailConfig {
            mailto: vec![email.into()],
            ..Default::default()
        },
    };

    endpoint.send(&notification)?;

    Ok(())
}

/// Summary of a successful Tape Job
#[derive(Default)]
pub struct TapeBackupJobSummary {
    /// The list of snaphots backed up
    pub snapshot_list: Vec<String>,
    /// The total time of the backup job
    pub duration: std::time::Duration,
    /// The labels of the used tapes of the backup job
    pub used_tapes: Option<Vec<String>>,
}

fn send_job_status_mail(email: &str, subject: &str, text: &str) -> Result<(), Error> {
    let (config, _) = crate::config::node::config()?;
    let from = config.email_from;

    // NOTE: some (web)mailers have big problems displaying text mails, so include html as well
    let escaped_text = handlebars::html_escape(text);
    let html = format!("<html><body><pre>\n{escaped_text}\n<pre>");

    let nodename = proxmox_sys::nodename();

    let author = format!("Proxmox Backup Server - {nodename}");

    sendmail(
        &[email],
        subject,
        Some(text),
        Some(&html),
        from.as_deref(),
        Some(&author),
    )?;

    Ok(())
}

pub fn send_gc_status(
    datastore: &str,
    status: &GarbageCollectionStatus,
    result: &Result<(), Error>,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "datastore": datastore,
        "fqdn": fqdn,
        "port": port,
    });

    let (severity, template) = match result {
        Ok(()) => {
            let deduplication_factor = if status.disk_bytes > 0 {
                (status.index_data_bytes as f64) / (status.disk_bytes as f64)
            } else {
                1.0
            };

            data["status"] = json!(status);
            data["deduplication-factor"] = format!("{:.2}", deduplication_factor).into();

            (Severity::Info, "gc-ok")
        }
        Err(err) => {
            data["error"] = err.to_string().into();
            (Severity::Error, "gc-err")
        }
    };
    let metadata = HashMap::from([
        ("datastore".into(), datastore.into()),
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "gc".into()),
    ]);

    let notification = Notification::from_template(severity, template, data, metadata);

    let (email, notify, mode) = lookup_datastore_notify_settings(datastore);
    match mode {
        NotificationMode::LegacySendmail => {
            let notify = notify.gc.unwrap_or(Notify::Always);

            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }

            if let Some(email) = email {
                send_sendmail_legacy_notification(notification, &email)?;
            }
        }
        NotificationMode::NotificationSystem => {
            send_notification(notification)?;
        }
    }

    Ok(())
}

pub fn send_verify_status(
    email: &str,
    notify: DatastoreNotify,
    job: VerificationJobConfig,
    result: &Result<Vec<String>, Error>,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "job": job,
        "fqdn": fqdn,
        "port": port,
    });

    let mut result_is_ok = false;

    let text = match result {
        Ok(errors) if errors.is_empty() => {
            result_is_ok = true;
            HANDLEBARS.render("verify_ok_template", &data)?
        }
        Ok(errors) => {
            data["errors"] = json!(errors);
            HANDLEBARS.render("verify_err_template", &data)?
        }
        Err(_) => {
            // aborted job - do not send any email
            return Ok(());
        }
    };

    match notify.verify {
        None => { /* send notifications by default */ }
        Some(notify) => {
            if notify == Notify::Never || (result_is_ok && notify == Notify::Error) {
                return Ok(());
            }
        }
    }

    let subject = match result {
        Ok(errors) if errors.is_empty() => format!("Verify Datastore '{}' successful", job.store),
        _ => format!("Verify Datastore '{}' failed", job.store),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

pub fn send_prune_status(
    store: &str,
    jobname: &str,
    result: &Result<(), Error>,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "jobname": jobname,
        "store": store,
        "fqdn": fqdn,
        "port": port,
    });

    let (template, severity) = match result {
        Ok(()) => ("prune-ok", Severity::Info),
        Err(err) => {
            data["error"] = err.to_string().into();
            ("prune-err", Severity::Error)
        }
    };

    let metadata = HashMap::from([
        ("job-id".into(), jobname.to_string()),
        ("datastore".into(), store.into()),
        ("hostname".into(), proxmox_sys::nodename().into()),
        ("type".into(), "prune".into()),
    ]);

    let notification = Notification::from_template(severity, template, data, metadata);

    let (email, notify, mode) = lookup_datastore_notify_settings(store);
    match mode {
        NotificationMode::LegacySendmail => {
            let notify = notify.prune.unwrap_or(Notify::Error);

            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }

            if let Some(email) = email {
                send_sendmail_legacy_notification(notification, &email)?;
            }
        }
        NotificationMode::NotificationSystem => {
            send_notification(notification)?;
        }
    }

    Ok(())
}

pub fn send_sync_status(
    email: &str,
    notify: DatastoreNotify,
    job: &SyncJobConfig,
    result: &Result<(), Error>,
) -> Result<(), Error> {
    match notify.sync {
        None => { /* send notifications by default */ }
        Some(notify) => {
            if notify == Notify::Never || (result.is_ok() && notify == Notify::Error) {
                return Ok(());
            }
        }
    }

    let (fqdn, port) = get_server_url();
    let mut data = json!({
        "job": job,
        "fqdn": fqdn,
        "port": port,
    });

    let text = match result {
        Ok(()) => HANDLEBARS.render("sync_ok_template", &data)?,
        Err(err) => {
            data["error"] = err.to_string().into();
            HANDLEBARS.render("sync_err_template", &data)?
        }
    };

    let tmp_src_string;
    let source_str = if let Some(remote) = &job.remote {
        tmp_src_string = format!("Sync remote '{}'", remote);
        &tmp_src_string
    } else {
        "Sync local"
    };

    let subject = match result {
        Ok(()) => format!("{} datastore '{}' successful", source_str, job.remote_store,),
        Err(_) => format!("{} datastore '{}' failed", source_str, job.remote_store,),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

pub fn send_tape_backup_status(
    email: &str,
    id: Option<&str>,
    job: &TapeBackupJobSetup,
    result: &Result<(), Error>,
    summary: TapeBackupJobSummary,
) -> Result<(), Error> {
    let (fqdn, port) = get_server_url();
    let duration: proxmox_time::TimeSpan = summary.duration.into();
    let mut data = json!({
        "job": job,
        "fqdn": fqdn,
        "port": port,
        "id": id,
        "snapshot-list": summary.snapshot_list,
        "used-tapes": summary.used_tapes,
        "duration": duration.to_string(),
    });

    let text = match result {
        Ok(()) => HANDLEBARS.render("tape_backup_ok_template", &data)?,
        Err(err) => {
            data["error"] = err.to_string().into();
            HANDLEBARS.render("tape_backup_err_template", &data)?
        }
    };

    let subject = match (result, id) {
        (Ok(()), Some(id)) => format!("Tape Backup '{id}' datastore '{}' successful", job.store,),
        (Ok(()), None) => format!("Tape Backup datastore '{}' successful", job.store,),
        (Err(_), Some(id)) => format!("Tape Backup '{id}' datastore '{}' failed", job.store,),
        (Err(_), None) => format!("Tape Backup datastore '{}' failed", job.store,),
    };

    send_job_status_mail(email, &subject, &text)?;

    Ok(())
}

/// Send email to a person to request a manual media change
pub fn send_load_media_email(
    changer: bool,
    device: &str,
    label_text: &str,
    to: &str,
    reason: Option<String>,
) -> Result<(), Error> {
    use std::fmt::Write as _;

    let device_type = if changer { "changer" } else { "drive" };

    let subject = format!("Load Media '{label_text}' request for {device_type} '{device}'");

    let mut text = String::new();

    if let Some(reason) = reason {
        let _ = write!(
            text,
            "The {device_type} has the wrong or no tape(s) inserted. Error:\n{reason}\n\n"
        );
    }

    if changer {
        text.push_str("Please insert the requested media into the changer.\n\n");
        let _ = writeln!(text, "Changer: {device}");
    } else {
        text.push_str("Please insert the requested media into the backup drive.\n\n");
        let _ = writeln!(text, "Drive: {device}");
    }
    let _ = writeln!(text, "Media: {label_text}");

    send_job_status_mail(to, &subject, &text)
}

fn get_server_url() -> (String, usize) {
    // user will surely request that they can change this

    let nodename = proxmox_sys::nodename();
    let mut fqdn = nodename.to_owned();

    if let Ok(resolv_conf) = crate::api2::node::dns::read_etc_resolv_conf() {
        if let Some(search) = resolv_conf["search"].as_str() {
            fqdn.push('.');
            fqdn.push_str(search);
        }
    }

    let port = 8007;

    (fqdn, port)
}

pub fn send_updates_available(updates: &[&APTUpdateInfo]) -> Result<(), Error> {
    // update mails always go to the root@pam configured email..
    if let Some(email) = lookup_user_email(Userid::root_userid()) {
        let nodename = proxmox_sys::nodename();
        let subject = format!("New software packages available ({nodename})");

        let (fqdn, port) = get_server_url();

        let text = HANDLEBARS.render(
            "package_update_template",
            &json!({
                "fqdn": fqdn,
                "port": port,
                "updates": updates,
            }),
        )?;

        send_job_status_mail(&email, &subject, &text)?;
    }
    Ok(())
}

/// send email on certificate renewal failure.
pub fn send_certificate_renewal_mail(result: &Result<(), Error>) -> Result<(), Error> {
    let error: String = match result {
        Err(e) => e.to_string(),
        _ => return Ok(()),
    };

    if let Some(email) = lookup_user_email(Userid::root_userid()) {
        let (fqdn, port) = get_server_url();

        let text = HANDLEBARS.render(
            "certificate_renewal_err_template",
            &json!({
                "fqdn": fqdn,
                "port": port,
                "error": error,
            }),
        )?;

        let subject = "Could not renew certificate";

        send_job_status_mail(&email, subject, &text)?;
    }

    Ok(())
}

/// Lookup users email address
pub fn lookup_user_email(userid: &Userid) -> Option<String> {
    if let Ok(user_config) = pbs_config::user::cached_config() {
        if let Ok(user) = user_config.lookup::<User>("user", userid.as_str()) {
            return user.email;
        }
    }

    None
}

/// Lookup Datastore notify settings
pub fn lookup_datastore_notify_settings(
    store: &str,
) -> (Option<String>, DatastoreNotify, NotificationMode) {
    let mut email = None;

    let notify = DatastoreNotify {
        gc: None,
        verify: None,
        sync: None,
        prune: None,
    };

    let (config, _digest) = match pbs_config::datastore::config() {
        Ok(result) => result,
        Err(_) => return (email, notify, NotificationMode::default()),
    };

    let config: DataStoreConfig = match config.lookup("datastore", store) {
        Ok(result) => result,
        Err(_) => return (email, notify, NotificationMode::default()),
    };

    email = match config.notify_user {
        Some(ref userid) => lookup_user_email(userid),
        None => lookup_user_email(Userid::root_userid()),
    };

    let notification_mode = config.notification_mode.unwrap_or_default();
    let notify_str = config.notify.unwrap_or_default();

    if let Ok(value) = DatastoreNotify::API_SCHEMA.parse_property_string(&notify_str) {
        if let Ok(notify) = serde_json::from_value(value) {
            return (email, notify, notification_mode);
        }
    }

    (email, notify, notification_mode)
}

#[test]
fn test_template_register() {
    assert!(HANDLEBARS.has_template("verify_ok_template"));
    assert!(HANDLEBARS.has_template("verify_err_template"));

    assert!(HANDLEBARS.has_template("sync_ok_template"));
    assert!(HANDLEBARS.has_template("sync_err_template"));

    assert!(HANDLEBARS.has_template("tape_backup_ok_template"));
    assert!(HANDLEBARS.has_template("tape_backup_err_template"));

    assert!(HANDLEBARS.has_template("package_update_template"));

    assert!(HANDLEBARS.has_template("certificate_renewal_err_template"));
}
