use libsql_client::{QueryResult, Statement, Value};
use worker::*;

mod utils;

// Log each request to dev console
fn log_request(req: &Request) {
    console_log!(
        "{} - [{}], located at: {:?}, within: {}",
        Date::now().to_string(),
        req.path(),
        req.cf().coordinates().unwrap_or_default(),
        req.cf().region().unwrap_or_else(|| "unknown region".into())
    );
}

// Take a query result and render it into a HTML table
fn result_to_html_table(result: QueryResult) -> String {
    let mut html = "<table style=\"border: 1px solid\">".to_string();
    match result {
        QueryResult::Error((msg, _)) => return format!("Error: {msg}"),
        QueryResult::Success((result, _)) => {
            for column in &result.columns {
                html += &format!("<th style=\"border: 1px solid\">{column}</th>");
            }
            for row in result.rows {
                html += "<tr style=\"border: 1px solid\">";
                for column in &result.columns {
                    html += &format!("<td>{}</td>", row.cells[column]);
                }
                html += "</tr>";
            }
        }
    };
    html += "</table>";
    html
}

// Create a javascript canvas which loads a map of visited airports
fn create_map_canvas(result: QueryResult) -> String {
    let mut canvas = r#"
  <script src="https://cdnjs.cloudflare.com/ajax/libs/p5.js/0.5.16/p5.min.js" type="text/javascript"></script>
  <script src="https://unpkg.com/mappa-mundi/dist/mappa.js" type="text/javascript"></script>
    <script>
    let myMap;
    let canvas;
    const mappa = new Mappa('Leaflet');
    const options = {
      lat: 0,
      lng: 0,
      zoom: 2,
      style: "http://{s}.tile.osm.org/{z}/{x}/{y}.png"
    }

    function setup(){
      canvas = createCanvas(640,480);
      myMap = mappa.tileMap(options); 
      myMap.overlay(canvas) 
    
      fill(200, 100, 100);
      myMap.onChange(drawPoint);
    }

    function draw(){
    }

    function drawPoint(){
      clear();
      let point;"#.to_owned();

    match result {
        QueryResult::Error((msg, _)) => console_log!("Error: {}", msg),
        QueryResult::Success((result, _)) => {
            for row in result.rows {
                canvas += &format!(
                    "point = myMap.latLngToPixel({}, {});\nellipse(point.x, point.y, 10, 10);\ntext({}, point.x, point.y);\n",
                    row.cells["lat"], row.cells["long"], row.cells["airport"]
                );
            }
        }
    };
    canvas += "}</script>";
    canvas
}

// Serve a request to load the page
async fn serve(req: Request, db: impl libsql_client::Connection) -> anyhow::Result<String> {
    // Recreate the tables if they do not exist yet
    db.execute("CREATE TABLE IF NOT EXISTS counter(country TEXT, city TEXT, value, PRIMARY KEY(country, city)) WITHOUT ROWID")
    .await
    .ok();
    db.execute(
        "CREATE TABLE IF NOT EXISTS coordinates(lat INT, long INT, airport TEXT, PRIMARY KEY (lat, long))",
    )
    .await
    .ok();

    // Update the tables - the per-city counter and airport coordinates
    let cf = req.cf();
    let airport = cf.colo();
    let country = cf.country().unwrap_or_default();
    let city = cf.city().unwrap_or_default();
    let coordinates = cf.coordinates().unwrap_or_default();
    db.transaction([
        Statement::with_params(
            "INSERT INTO counter VALUES (?, ?, 0)",
            &[country.as_str(), city.as_str()],
        ),
        Statement::with_params(
            "UPDATE counter SET value = value + 1 WHERE country = ? AND city = ?",
            &[country, city],
        ),
        Statement::with_params(
            "INSERT INTO coordinates VALUES (?, ?, ?)",
            &[
                Value::Real(coordinates.0 as f64),
                Value::Real(coordinates.1 as f64),
                airport.into(),
            ],
        ),
    ])
    .await
    .ok();

    let counter_response = db.execute("SELECT * FROM counter").await?;
    let scoreboard = result_to_html_table(counter_response);

    let canvas = create_map_canvas(
        db.execute("SELECT airport, lat, long FROM coordinates")
            .await?,
    );
    let html = format!("{canvas} Database powered by <a href=\"https://chiselstrike.com/\">Turso</a>. <br /> Scoreboard: <br /> {scoreboard} <footer>Map data from OpenStreetMap (https://tile.osm.org/)</footer>");
    Ok(html)
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    log_request(&req);

    utils::set_panic_hook();
    let router = Router::new();

    router
        .get_async("/", |req, ctx| async move {
            let db = match libsql_client::workers::Connection::connect_from_ctx(&ctx) {
                Ok(db) => db,
                Err(e) => {
                    console_log!("Error {e}");
                    return Response::from_html(format!("Error establishing connection: {e}"));
                }
            };
            match serve(req, db).await {
                Ok(html) => Response::from_html(html),
                Err(e) => Err(Error::from(format!("{e}"))),
            }
        })
        .get("/worker-version", |_, ctx| {
            let version = ctx.var("WORKERS_RS_VERSION")?.to_string();
            Response::ok(version)
        })
        .run(req, env)
        .await
}
