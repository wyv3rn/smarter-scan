use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use bjw_db_derive::derive_bjw_db;
use chrono::DateTime;
use clap::Parser;
use maud::{Markup, html};
use serde::{Deserialize, Serialize};

type ScanId = u64;

#[derive(Serialize, Deserialize, Clone, Copy)]
enum Status {
    Init,
    Failed,
    ScanningFront,
    FrontDone,
    ScanningBack,
    BackDone,
    Postprocessing,
    Done,
}

#[derive(Serialize, Deserialize, Clone)]
struct Scan {
    id: ScanId,
    id_as_utc: String,
    path: PathBuf,
    status: Status,
}

#[derive(Default, Serialize, Deserialize, Clone)]
struct Scans {
    scans: BTreeMap<ScanId, Scan>,
}

#[derive_bjw_db(thread_safe)]
impl Scans {
    pub fn insert_scan(&mut self, scan: Scan) {
        self.scans.insert(scan.id, scan);
    }

    pub fn get_scan(&self, id: &ScanId) -> Option<Scan> {
        self.scans.get(id).cloned()
    }

    pub fn get_status(&self, id: &ScanId) -> Option<Status> {
        self.scans.get(id).map(|s| s.status)
    }

    pub fn set_status(&mut self, id: ScanId, s: Status) {
        if let Some(scan) = self.scans.get_mut(&id) {
            scan.status = s
        }
    }
}

fn create_new_scan(db: Arc<ScanDb>) -> std::io::Result<Scan> {
    let id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let id_as_utc = DateTime::from_timestamp(id as i64, 0).unwrap().to_string();
    let dir = id_as_utc.replace(" ", "-");
    let path = db.path().clone().join(dir);
    std::fs::create_dir(&path)?;
    let scan = Scan {
        id,
        id_as_utc,
        path,
        status: Status::Init,
    };
    db.insert_scan(scan.clone())?;
    Ok(scan)
}

type ScanDb = ScansDb;
type StateDb = State<Arc<ScanDb>>;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// TCP address to bind on
    #[arg(short, long, default_value = "0.0.0.0:8080")]
    address: String,
    /// Path to database
    #[arg(short, long, default_value = "smarter-scan-db")]
    database: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    println!("Opening database at {}", args.database);
    let db = Arc::new(ScanDb::open(args.database).expect("Failed to open database"));
    let app = Router::new()
        .route("/", get(|| async { Redirect::to("/scans") }))
        .route("/scans", get(scans))
        .route("/scans/new", get(new_scan))
        .route("/scans/front/{id}", get(scan_front))
        .route("/scans/back/{id}", get(scan_back))
        .route("/scans/postproc/{id}", get(scan_postproc))
        .with_state(db);

    println!("Binding to address {}", args.address);
    let listener = tokio::net::TcpListener::bind(args.address).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn scans(State(_db): StateDb) -> Markup {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let now_utc = DateTime::from_timestamp(now as i64, 0).unwrap();
    html! {
        (head_with_refresh(5))
        h1 { "Smarter Scan" }
        hr {}
        p { a href="/scans/new" { "Neuer Scan" } }
        p { "Aktuelle Uhrzeit: " (now_utc) }
        hr {}
    }
}

async fn new_scan(State(db): StateDb) -> Response {
    match create_new_scan(db.clone()) {
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        Ok(scan) => html! {
            h1 { "Neuer Scan" }
            p { "Bitte Stapel mit Frontseiten nach oben einlegen, danach \"Weiter\" drücken" }
            p { a href={ "/scans/front/" (scan.id) } { "Weiter" } }
        }
        .into_response(),
    }
}

async fn scan_front(State(db): StateDb, Path(id): Path<ScanId>) -> Response {
    let scan = match db.get_scan(&id) {
        Some(s) => s,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let front_done = match scan.status {
        Status::Init => {
            start_scan_front(scan.id, db.clone());
            if db.set_status(scan.id, Status::ScanningFront).is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            false
        }
        Status::ScanningFront => false,
        Status::FrontDone => true,
        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if front_done {
        html! {
            h1 { "Scannen der Frontseiten" }
            p { "Scan der Frontseiten abgeschlossen. Entweder jetzt Rückseiten nach oben einlegen (und \"Weiter\" drücken), oder \"Fertig\" drücken, falls keine Rückseiten vorhanden sind." }
            p { a href={ "/scans/back/" (scan.id) } { "Weiter" } }
            p { a href={ "/scans/postproc/" (scan.id) } { "Fertig" } }
        }.into_response()
    } else {
        html! {
            (head_with_refresh(1))
            h1 { "Scannen der Frontseiten" }
            p { "Bitte warten ..." }
        }
        .into_response()
    }
}

async fn scan_back(State(db): StateDb, Path(id): Path<ScanId>) -> Response {
    let scan = match db.get_scan(&id) {
        Some(s) => s,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let back_done = match scan.status {
        Status::FrontDone => {
            start_scan_back(scan.id, db.clone());
            if db.set_status(scan.id, Status::ScanningBack).is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            false
        }
        Status::ScanningBack => false,
        Status::BackDone => true,
        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if back_done {
        html! {
            h1 { "Scannen der Rückseiten" }
            p { "Abgeschlossen, bitte \"Weiter\" drücken" }
            p { a href={ "/scans/postproc/" (scan.id) } { "Weiter" } }
        }
        .into_response()
    } else {
        html! {
            (head_with_refresh(1))
            h1 { "Scannen der Rückseiten" }
            p { "Bitte warten ..." }
        }
        .into_response()
    }
}

async fn scan_postproc(State(db): StateDb, Path(id): Path<ScanId>) -> Response {
    let scan = match db.get_scan(&id) {
        Some(s) => s,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let postproc_done = match scan.status {
        Status::FrontDone | Status::BackDone => {
            start_post_proc(scan.id, db.clone());
            if db.set_status(scan.id, Status::Postprocessing).is_err() {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            false
        }
        Status::Postprocessing => false,
        Status::Done => true,
        _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    if postproc_done {
        html! {
            h1 { "Nachbearbeitung" }
            p { "Abgeschlossen, bitte \"Weiter\" drücken" }
            p { a href={ "/scans" } { "Weiter" } }
        }
        .into_response()
    } else {
        html! {
            (head_with_refresh(1))
            h1 { "Nachbearbeitung" }
            p { "Bitte warten ..." }
        }
        .into_response()
    }
}

fn head_with_refresh(interval: u16) -> Markup {
    html! {
        head {
            meta http-equiv="refresh" content=((interval)) {}
        }
    }
}

fn start_scan_front(id: ScanId, db: Arc<ScanDb>) {
    std::thread::spawn(move || {
        db.set_status(id, Status::FrontDone).unwrap();
    });
}

fn start_scan_back(id: ScanId, db: Arc<ScanDb>) {
    std::thread::spawn(move || {
        db.set_status(id, Status::BackDone).unwrap();
    });
}

fn start_post_proc(id: ScanId, db: Arc<ScanDb>) {
    std::thread::spawn(move || {
        db.set_status(id, Status::Done).unwrap();
    });
}
