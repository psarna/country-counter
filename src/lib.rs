use libsql_client::DatabaseClient;
use libsql_client::{args, workers::Client, ResultSet, Statement};
use std::collections::HashMap;
use worker::*;

mod utils;

// Log each request to dev console
fn log_request(req: &Request) {
    tracing::info!(
        "[{}], located at: {:?}, within: {}",
        req.path(),
        req.cf().coordinates().unwrap_or_default(),
        req.cf().region().unwrap_or_else(|| "unknown region".into())
    );
}

// Take a query result and render it into a HTML table
fn result_to_html_table(result: ResultSet) -> String {
    let mut html = "<table style=\"border: 1px solid\">".to_string();
    for column in result.columns {
        html += &format!("<th style=\"border: 1px solid\">{column}</th>");
    }
    for row in result.rows {
        html += "<tr style=\"border: 1px solid\">";
        for cell in row.values {
            html += &format!("<td>{cell}</td>");
        }
        html += "</tr>";
    }
    html += "</table>";
    html
}

// Create a javascript canvas which loads a map of visited airports
fn create_map_canvas(result: ResultSet) -> String {
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

    for row in result.rows {
        canvas += &format!(
            "point = myMap.latLngToPixel({}, {});\nellipse(point.x, point.y, 10, 10);\ntext({}, point.x, point.y);\n",
            // NOTICE: value_map is not very efficient and only enabled if the feature "mapping_names_to_values_in_rows" is enabled
            row.value_map["lat"], row.value_map["long"], row.value_map["airport"]
        );
    }
    canvas += "}</script>";
    canvas
}

// Serve a request to load the page
async fn serve(
    airport: impl Into<String>,
    country: impl Into<String>,
    city: impl Into<String>,
    coordinates: (f32, f32),
    db: &impl libsql_client::DatabaseClient,
) -> anyhow::Result<String> {
    let airport = airport.into();
    let country = country.into();
    let city = city.into();

    // Recreate the tables if they do not exist yet
    if let Err(e) = db.execute("CREATE TABLE IF NOT EXISTS counter(country TEXT, city TEXT, value, PRIMARY KEY(country, city)) WITHOUT ROWID")
    .await {
        tracing::error!("Error creating table: {e}");
        anyhow::bail!("{e}")
    };
    if let Err(e) = db.execute(
        "CREATE TABLE IF NOT EXISTS coordinates(lat INT, long INT, airport TEXT, PRIMARY KEY (lat, long))",
    )
    .await {
        tracing::error!("Error creating table: {e}");
        anyhow::bail!("{e}")
    };
    let tx = db.transaction().await?;
    tx.execute(Statement::with_args(
        "INSERT OR IGNORE INTO counter VALUES (?, ?, 0)",
        // Parameters that have a single type can be passed as a regular slice
        &[&country, &city],
    ))
    .await?;
    tx.execute(Statement::with_args(
        "UPDATE counter SET value = value + 1 WHERE country = ? AND city = ?",
        &[country, city],
    ))
    .await?;
    tx.execute(Statement::with_args(
        "INSERT OR IGNORE INTO coordinates VALUES (?, ?, ?)",
        // Parameters with different types can be passed to a convenience macro - args!()
        args!(coordinates.0, coordinates.1, airport),
    ))
    .await?;
    tx.commit().await?;

    let counter_response = db.execute("SELECT * FROM counter").await?;
    let scoreboard = result_to_html_table(counter_response);

    let canvas = create_map_canvas(
        db.execute("SELECT airport, lat, long FROM coordinates")
            .await?,
    );
    let html = format!(
        r#"
        <body>
        {canvas} Database powered by <a href="https://chiselstrike.com/">Turso</a>.
        <br /> Scoreboard: <br /> {scoreboard}
        <footer>Map data from OpenStreetMap (https://tile.osm.org/)</footer>
        </body>
        "#
    );
    Ok(html)
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    log_request(&req);

    utils::set_panic_hook();
    let router = Router::new();

    tracing_worker::init(&env);

    router
        .get_async("/", |req, ctx| async move {
            let db = match libsql_client::workers::Client::from_ctx(&ctx).await {
                Ok(db) => db,
                Err(e) => {
                    tracing::error!("Error {e}");
                    return Response::from_html(format!("Error establishing connection: {e}"));
                }
            };
            let cf = req.cf();
            let airport = cf.colo();
            let country = cf.country().unwrap_or_default();
            let city = cf.city().unwrap_or_default();
            let coordinates = cf.coordinates().unwrap_or_default();
            match serve(airport, country, city, coordinates, &db).await {
                Ok(html) => Response::from_html(html),
                Err(e) => Response::ok(format!("Error: {e}")),
            }
        })
        .get("/worker-version", |_, ctx| {
            let version = ctx.var("WORKERS_RS_VERSION")?.to_string();
            Response::ok(version)
        })
        .get("/locate", |req, _ctx| {
            let cf = req.cf();
            let airport = cf.colo();
            let country = cf.country().unwrap_or_default();
            let city = cf.city().unwrap_or_default();
            let coordinates = cf.coordinates().unwrap_or_default();
            Response::ok(format!(
                "{};{};{};{};{}",
                airport, country, city, coordinates.0, coordinates.1
            ))
        })
        .get_async("/users", |_, ctx| async move {
            let client = match Client::from_ctx(&ctx).await {
                Ok(client) => client,
                Err(e) => return Response::error(e.to_string(), 500),
            };

            let stmt = "select * from example_users";
            let rs = match client.execute(stmt).await {
                Ok(rs) => rs,
                Err(e) => return Response::error(e.to_string(), 500),
            };

            Response::from_json(&serde_json::json!(rs))
        })
        .get_async("/add-user", |req, ctx| async move {
            let url = req.url().unwrap();
            let hash_query: HashMap<String, String> = url.query_pairs().into_owned().collect();
            let email = match hash_query.get("email") {
                Some(string) => string,
                None => return Response::error("No email", 400),
            };

            let client = match libsql_client::workers::Client::from_ctx(&ctx).await {
                Ok(client) => client,
                Err(e) => return Response::error(e.to_string(), 500),
            };

            let stmt = Statement::with_args("insert into example_users values (?)", args!(email));
            match client.execute(stmt).await {
                Ok(_) => Response::from_json(&serde_json::json!({
                    "result": "Added"
                })),
                Err(e) => Response::error(e.to_string(), 500),
            }
        })
        .run(req, env)
        .await
}

#[cfg(test)]
mod tests {
    use libsql_client::{DatabaseClient, ResultSet, Value};
    fn test_db() -> libsql_client::local::Client {
        libsql_client::local::Client::in_memory().unwrap()
    }

    #[tokio::test]
    async fn test_counter_updated() {
        let db = test_db();

        let payloads = [
            ("waw", "PL", "Warsaw", (52.1672, 20.9679)),
            ("waw", "PL", "Warsaw", (52.1672, 20.9679)),
            ("waw", "PL", "Warsaw", (52.1672, 20.9679)),
            ("hel", "FI", "Helsinki", (60.3183, 24.9497)),
            ("hel", "FI", "Helsinki", (60.3183, 24.9497)),
        ];

        for p in payloads {
            super::serve(p.0, p.1, p.2, p.3, &db).await.unwrap();
        }

        let ResultSet { columns, rows } = db
            .execute("SELECT country, city, value FROM counter")
            .await
            .unwrap()
            .into_result_set()
            .unwrap();

        assert_eq!(columns, vec!["country", "city", "value"]);
        for row in rows {
            let city = match &row.cells["city"] {
                Value::Text(c) => c.as_str(),
                _ => panic!("Invalid entry for a city: {:?}", row),
            };
            match city {
                "Warsaw" => assert_eq!(row.cells["value"], 3.into()),
                "Helsinki" => assert_eq!(row.cells["value"], 2.into()),
                _ => panic!("Unknown city: {:?}", row),
            }
        }
    }
}
