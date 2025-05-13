use std::{
    collections::BTreeMap,
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

#[derive(Serialize, Deserialize, Clone)]
struct Scan {
    id: ScanId,
    id_as_utc: String,
    dir: String,
}

#[derive(Default, Serialize, Deserialize, Clone)]
struct Scans {
    scans: BTreeMap<ScanId, Scan>,
}

#[derive_bjw_db(thread_safe)]
impl Scans {
    pub fn create_new_scan(&mut self) -> Scan {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let id_as_utc = DateTime::from_timestamp(id as i64, 0).unwrap().to_string();
        let dir = id_as_utc.replace(" ", "-");
        let scan = Scan { id, id_as_utc, dir };
        self.scans.insert(id, scan.clone());
        scan
    }

    pub fn get_scan(&self, id: &ScanId) -> Option<Scan> {
        self.scans.get(id).cloned()
    }
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
    match db.create_new_scan() {
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        Ok(scan) => html! {
            h1 { "Neuer Scan" }
            p { "Bitte Stapel mit Frontseiten nach unten einlegen, danach \"Weiter\" drücken" }
            p { a href={ "/scans/front/" (scan.id) } { "Weiter" } }
        }
        .into_response(),
    }
}

async fn scan_front(State(db): StateDb, Path(id): Path<ScanId>) -> Response {
    // TODO check status each time
    match db.get_scan(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(scan) => html! {
            (head_with_refresh(1))
            h1 { "Scannen der Frontseiten" }
            p { "..." }
            p { "Scan der Frontseiten abgeschlossen. Entweder jetzt Rückseiten nach unten einlegen (und \"Weiter\" drücken), oder \"Fertig\" drücken, falls keine Rückseiten vorhanden sind." }
            p { a href={ "/scans/back/" (scan.id) } { "Weiter" } }
            p { a href={ "/scans/postproc/" (scan.id) } { "Fertig" } }
        }
        .into_response(),
    }
}

async fn scan_back(State(db): StateDb, Path(id): Path<ScanId>) -> Response {
    // TODO check status each time, auto redirect to post when finished
    match db.get_scan(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(scan) => html! {
            (head_with_refresh(1))
            h1 { "Scannen der Rückseiten" }
            p { "..." }
            p { "Bitte \"Fertig\" drücken" }
            p { a href={ "/scans/postproc/" (scan.id) } { "Fertig" } }
        }
        .into_response(),
    }
}

async fn scan_postproc(State(db): StateDb, Path(id): Path<ScanId>) -> Response {
    // TODO check status each time, auto redirect to landing page when finished
    match db.get_scan(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(_scan) => html! {
            (head_with_refresh(1))
            h1 { "Nachbearbeitung des Scans" }
            p { "..." }
            p { "Bitte \"Fertig\" drücken" }
            p { a href="/scans" { "Fertig" } }
        }
        .into_response(),
    }
}

fn head_with_refresh(interval: u16) -> Markup {
    html! {
        head {
            meta http-equiv="refresh" content=((interval)) {}
        }
    }
}
