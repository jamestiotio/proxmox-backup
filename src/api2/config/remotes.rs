use failure::*;
use serde_json::Value;

use proxmox::api::{api, ApiMethod, Router, RpcEnvironment};

use crate::api2::types::*;
use crate::config::remotes;

#[api(
    input: {
        properties: {},
    },
    returns: {
        description: "The list of configured remotes.",
        type: Array,
        items: {
            type: remotes::Remote,
        },
    },
)]
/// List all remotes
pub fn list_remotes(
    _param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut dyn RpcEnvironment,
) -> Result<Value, Error> {

    let (config, digest) = remotes::config()?;

    Ok(config.convert_to_array("name", Some(&digest)))
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            host: {
                schema: DNS_NAME_OR_IP_SCHEMA,
            },
            userid: {
                schema: PROXMOX_USER_ID_SCHEMA,
            },
            password: {
                schema: remotes::REMOTE_PASSWORD_SCHEMA,
            },
        },
    },
)]
/// Create new remote.
pub fn create_remote(name: String, param: Value) -> Result<(), Error> {

    // fixme: locking ?

    let remote: remotes::Remote = serde_json::from_value(param.clone())?;

    let (mut config, _digest) = remotes::config()?;

    if let Some(_) = config.sections.get(&name) {
        bail!("remote '{}' already exists.", name);
    }

    config.set_data(&name, "remote", &remote)?;

    remotes::save_config(&config)?;

    Ok(())
}

#[api(
   input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
        },
    },
)]
/// Read remote configuration data.
pub fn read_remote(name: String) -> Result<Value, Error> {
    let (config, digest) = remotes::config()?;
    let mut data = config.lookup_json("remote", &name)?;
    data.as_object_mut().unwrap()
        .insert("digest".into(), proxmox::tools::digest_to_hex(&digest).into());
    Ok(data)
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
            comment: {
                optional: true,
                schema: SINGLE_LINE_COMMENT_SCHEMA,
            },
            host: {
                optional: true,
                schema: DNS_NAME_OR_IP_SCHEMA,
            },
            userid: {
                optional: true,
               schema: PROXMOX_USER_ID_SCHEMA,
            },
            password: {
                optional: true,
                schema: remotes::REMOTE_PASSWORD_SCHEMA,
            },
        },
    },
)]
/// Update remote configuration.
pub fn update_remote(
    name: String,
    comment: Option<String>,
    host: Option<String>,
    userid: Option<String>,
    password: Option<String>,
) -> Result<(), Error> {

    // fixme: locking ?
    // pass/compare digest
    let (mut config, _digest) = remotes::config()?;

    let mut data: remotes::Remote = config.lookup("remote", &name)?;

    if let Some(comment) = comment {
        let comment = comment.trim().to_string();
        if comment.is_empty() {
            data.comment = None;
        } else {
            data.comment = Some(comment);
        }
    }
    if let Some(host) = host { data.host = host; }
    if let Some(userid) = userid { data.userid = userid; }
    if let Some(password) = password { data.password = password; }

    config.set_data(&name, "remote", &data)?;

    remotes::save_config(&config)?;

    Ok(())
}

#[api(
    protected: true,
    input: {
        properties: {
            name: {
                schema: REMOTE_ID_SCHEMA,
            },
        },
    },
)]
/// Remove a remote from the configuration file.
pub fn delete_remote(name: String) -> Result<(), Error> {

    // fixme: locking ?
    // fixme: check digest ?

    let (mut config, _digest) = remotes::config()?;

    match config.sections.get(&name) {
        Some(_) => { config.sections.remove(&name); },
        None => bail!("remote '{}' does not exist.", name),
    }

    Ok(())
}

const ITEM_ROUTER: Router = Router::new()
    .get(&API_METHOD_READ_REMOTE)
    .put(&API_METHOD_UPDATE_REMOTE)
    .delete(&API_METHOD_DELETE_REMOTE);

pub const ROUTER: Router = Router::new()
    .get(&API_METHOD_LIST_REMOTES)
    .post(&API_METHOD_CREATE_REMOTE)
    .match_all("name", &ITEM_ROUTER);
