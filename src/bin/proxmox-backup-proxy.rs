extern crate proxmox_backup;

use proxmox_backup::api::router::*;
use proxmox_backup::api::config::*;
use proxmox_backup::server::rest::*;
use proxmox_backup::auth_helpers::*;

//use failure::*;
use lazy_static::lazy_static;

use futures::future::Future;

use hyper;

fn main() {

    if let Err(err) = syslog::init(
        syslog::Facility::LOG_DAEMON,
        log::LevelFilter::Info,
        Some("proxmox-backup-proxy")) {
        eprintln!("unable to inititialize syslog: {}", err);
        std::process::exit(-1);
    }

    let _ = public_auth_key(); // load with lazy_static
    let _ = csrf_secret(); // load with lazy_static

    let addr = ([0,0,0,0,0,0,0,0], 8007).into();

    lazy_static!{
       static ref ROUTER: Router = proxmox_backup::api2::router();
    }

    let mut config = ApiConfig::new(
        "/usr/share/javascript/proxmox-backup", &ROUTER, RpcEnvironmentType::PUBLIC);

    // add default dirs which includes jquery and bootstrap
    // my $base = '/usr/share/libpve-http-server-perl';
    // add_dirs($self->{dirs}, '/css/' => "$base/css/");
    // add_dirs($self->{dirs}, '/js/' => "$base/js/");
    // add_dirs($self->{dirs}, '/fonts/' => "$base/fonts/");
    config.add_alias("novnc", "/usr/share/novnc-pve");
    config.add_alias("extjs", "/usr/share/javascript/extjs");
    config.add_alias("fontawesome", "/usr/share/fonts-font-awesome");
    config.add_alias("xtermjs", "/usr/share/pve-xtermjs");
    config.add_alias("widgettoolkit", "/usr/share/javascript/proxmox-widget-toolkit");

    let rest_server = RestServer::new(config);

    let server = hyper::Server::bind(&addr)
        .serve(rest_server)
        .map_err(|e| eprintln!("server error: {}", e));


    // Run this server for... forever!
    hyper::rt::run(server);
}
