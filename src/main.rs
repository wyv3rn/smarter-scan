use std::{
    collections::BTreeMap,
    fs::FileType,
    path::PathBuf,
    process::{Command, ExitStatus},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    extract::{Path, State},
    http::{StatusCode, header},
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

    pub fn get_scans(&self) -> Vec<Scan> {
        self.scans.values().rev().cloned().collect()
    }

    pub fn get_scan(&self, id: &ScanId) -> Option<Scan> {
        self.scans.get(id).cloned()
    }

    pub fn get_status(&self, id: &ScanId) -> Option<Status> {
        self.scans.get(id).map(|s| s.status)
    }

    pub fn get_path(&self, id: &ScanId) -> Option<PathBuf> {
        self.scans.get(id).map(|s| s.path.clone())
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
        .route("/scans/download/{id}", get(download))
        .with_state(db);

    println!("Binding to address {}", args.address);
    let listener = tokio::net::TcpListener::bind(args.address).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn scans(State(db): StateDb) -> Markup {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let now_utc = DateTime::from_timestamp(now as i64, 0).unwrap();
    let scans = db.get_scans();
    html! {
        h1 { "Smarter Scan" }
        hr {}
        p { a href="/scans/new" { "Neuer Scan" } }
        p { "Aktuelle Uhrzeit: " (now_utc) }
        hr {}
        table {
            thead {
                tr {
                    th { "Scan ID" } th { "Steuerung" }
                }
            }
            tbody {
                @for scan in scans {
                    @let download_url = format!("/scans/download/{}", scan.id);
                    tr {
                        td { (scan.id_as_utc) }
                        td {
                            form action=(download_url) method="get" {
                                button { "Download" }
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn new_scan(State(db): StateDb) -> Response {
    match create_new_scan(db.clone()) {
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        Ok(scan) => html! {
            h1 { "Neuer Scan" }
            ul {
                li { "Bitte Stapel mit Frontseiten nach oben einlegen" }
                li { "Kopf der Seiten muss Richtung Heizung zeigen"}
                li { "Dritter und vierter Knopf von links müssen leuchten (grün, weiß)" }
                li { "Weiter drücken" }
            }
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
            p { "Scan der Frontseiten abgeschlossen. Jetzt gibt es zwei Optionen:" }
            ol {
                li { "Keine Rückseiten vorhanden => Fertig drücken" }
                li { "Rückseiten vorhanden" }
                ul {
                    li { "Stapel wieder einlegen, diesmal Rückseiten nach oben (also so wie der Stapel rauskam)" }
                    li { "Aber der Seitenkopf muss wieder Richtung Heizung zeigen => einmal drehen" }
                    li { "Warten, bis das rote X nicht mehr leuchtet" }
                    li { "Warten, bis dritter und vierter Knopf von links wieder leuchten (grün, weiß)" }
                    li { "Weiter drücken" }
                }
            }
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
        Redirect::to(&format!("/scans/postproc/{}", scan.id)).into_response()
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
        Redirect::to("/scans").into_response()
    } else {
        html! {
            (head_with_refresh(1))
            h1 { "Nachbearbeitung" }
            p { "Bitte warten ... (dauert schon ne Weile)" }
        }
        .into_response()
    }
}

async fn download(State(db): StateDb, Path(id): Path<ScanId>) -> Response {
    let scan = match db.get_scan(&id) {
        Some(s) => s,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    match tokio::fs::read(scan.path.join("scan.pdf")).await {
        Ok(contents) => (
            [
                (header::CONTENT_TYPE, "application/pdf"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"scan.pdf\"",
                ),
            ],
            contents,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
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
        if let Some(path) = db.get_path(&id) {
            if let Ok(output) = Command::new("smarter-scan-do")
                .arg(path)
                .arg("front")
                .output()
            {
                if ExitStatus::success(&output.status) {
                    db.set_status(id, Status::FrontDone).unwrap();
                } else {
                    println!("Scan front error log:");
                    println!("---------------------");
                    println!("{}", String::from_utf8(output.stderr).unwrap());
                    db.set_status(id, Status::Failed).unwrap();
                }
            } else {
                db.set_status(id, Status::Failed).unwrap();
            }
        }
    });
}

fn start_scan_back(id: ScanId, db: Arc<ScanDb>) {
    std::thread::spawn(move || {
        if let Some(path) = db.get_path(&id) {
            if let Ok(output) = Command::new("smarter-scan-do")
                .arg(path)
                .arg("back")
                .output()
            {
                if ExitStatus::success(&output.status) {
                    db.set_status(id, Status::BackDone).unwrap();
                } else {
                    println!("Scan back error log:");
                    println!("---------------------");
                    println!("{}", String::from_utf8(output.stderr).unwrap());
                    db.set_status(id, Status::Failed).unwrap();
                }
            } else {
                db.set_status(id, Status::Failed).unwrap();
            }
        }
    });
}

fn start_post_proc(id: ScanId, db: Arc<ScanDb>) {
    std::thread::spawn(move || {
        if let Some(path) = db.get_path(&id) {
            let mut cmd = Command::new("smarter-scan-post");
            cmd.arg("-o");
            cmd.arg(path.join("scan.pdf"));
            std::fs::read_dir(&path).unwrap().for_each(|p| {
                if let Ok(e) = p {
                    if let Ok(t) = e.file_type() {
                        if FileType::is_file(&t) {
                            cmd.arg(e.path());
                        }
                    }
                }
            });
            if let Ok(output) = cmd.output() {
                println!("Postprocess log:");
                println!("---------------");
                println!("{}", String::from_utf8(output.stdout).unwrap());
                if ExitStatus::success(&output.status) {
                    db.set_status(id, Status::Done).unwrap();
                } else {
                    db.set_status(id, Status::Failed).unwrap();
                }
            } else {
                db.set_status(id, Status::Failed).unwrap();
            }
        }
    });
}
