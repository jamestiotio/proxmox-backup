extern crate proxmox_backup;

use failure::*;

use proxmox_backup::tools;
use proxmox_backup::cli::*;
use proxmox_backup::api_schema::*;
use proxmox_backup::api_schema::router::*;

use serde_json::{Value};

use std::io::Write;
use std::path::PathBuf;

use proxmox_backup::pxar::encoder::*;
use proxmox_backup::pxar::decoder::*;

fn print_filenames(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let file = std::fs::File::open(archive)?;

    let mut reader = std::io::BufReader::new(file);

    let mut decoder = PxarDecoder::new(&mut reader);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut path = PathBuf::from(".");
    decoder.dump_entry(&mut path, false, &mut out)?;

    Ok(Value::Null)
}

fn dump_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let file = std::fs::File::open(archive)?;

    let mut reader = std::io::BufReader::new(file);

    let mut decoder = PxarDecoder::new(&mut reader);

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    println!("PXAR dump: {}", archive);

    let mut path = PathBuf::new();
    decoder.dump_entry(&mut path, true, &mut out)?;

    Ok(Value::Null)
}

fn create_archive(
    param: Value,
    _info: &ApiMethod,
    _rpcenv: &mut RpcEnvironment,
) -> Result<Value, Error> {

    let archive = tools::required_string_param(&param, "archive")?;
    let source = tools::required_string_param(&param, "source")?;
    let verbose = param["verbose"].as_bool().unwrap_or(false);
    let all_file_systems = param["all-file-systems"].as_bool().unwrap_or(false);

    let source = std::path::PathBuf::from(source);

    let mut dir = nix::dir::Dir::open(
        &source, nix::fcntl::OFlag::O_NOFOLLOW, nix::sys::stat::Mode::empty())?;

    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(archive)?;

    let mut writer = std::io::BufWriter::with_capacity(1024*1024, file);

    PxarEncoder::encode(source, &mut dir, &mut writer, all_file_systems, verbose)?;

    writer.flush()?;

    Ok(Value::Null)
}

fn main() {

    let cmd_def = CliCommandMap::new()
        .insert("create", CliCommand::new(
            ApiMethod::new(
                create_archive,
                ObjectSchema::new("Create new .pxar archive.")
                    .required("archive", StringSchema::new("Archive name"))
                    .required("source", StringSchema::new("Source directory."))
                    .optional("verbose", BooleanSchema::new("Verbose output.").default(false))
                    .optional("all-file-systems", BooleanSchema::new("Include mounted sudirs.").default(false))
           ))
            .arg_param(vec!["archive", "source"])
            .completion_cb("archive", tools::complete_file_name)
            .completion_cb("source", tools::complete_file_name)
           .into()
        )
        .insert("list", CliCommand::new(
            ApiMethod::new(
                print_filenames,
                ObjectSchema::new("List the contents of an archive.")
                    .required("archive", StringSchema::new("Archive name."))
            ))
            .arg_param(vec!["archive"])
            .completion_cb("archive", tools::complete_file_name)
            .into()
        )
        .insert("dump", CliCommand::new(
            ApiMethod::new(
                dump_archive,
                ObjectSchema::new("Textual dump of archive contents (debug toolkit).")
                    .required("archive", StringSchema::new("Archive name."))
            ))
            .arg_param(vec!["archive"])
            .completion_cb("archive", tools::complete_file_name)
            .into()
        );

    run_cli_command(cmd_def.into());
}
