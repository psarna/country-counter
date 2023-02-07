use libsql_client::{CellValue, QueryResult, Statement};
use worker::*;

mod utils;

trait Foo {
    fn some_method(&self);
}

struct Bar {
    thing: usize,
}

impl Foo for Bar {
    fn some_method(&self) {
        println!("self {}", self.thing);
    }
}

fn log_request(req: &Request) {
    console_log!(
        "{} - [{}], located at: {:?}, within: {}",
        Date::now().to_string(),
        req.path(),
        req.cf().coordinates().unwrap_or_default(),
        req.cf().region().unwrap_or_else(|| "unknown region".into())
    );
}

async fn serve(req: Request, db: libsql_client::Connection) -> Result<Response> {
    db.execute("CREATE TABLE IF NOT EXISTS counter(country TEXT, city TEXT, value, PRIMARY KEY(country, city)) WITHOUT ROWID")
    .await
    .ok();
    db.execute(
        "CREATE TABLE IF NOT EXISTS coordinates(lat INT, long INT, PRIMARY KEY (lat, long))",
    )
    .await
    .ok();

    //todo: transaction: bump, add lat + long
    let cf = req.cf();
    let airport = "x";//cf.colo();
    let country = "x";//cf.country().unwrap_or_default();
    let city = cf.city().unwrap_or_default();
    let coordinates = cf.coordinates().unwrap_or_default();
    console_log!("INFO {} {} {} {:?}", airport, country, city, coordinates);
    db.execute(format!(
        "INSERT INTO counter VALUES ('{}', 'region was used!')",
        req.cf().region().unwrap_or_else(|| "unknown region".into())
    ))
    .await
    .ok();

    let response = db
        .execute("SELECT * FROM counter WHERE key = 'turso'")
        .await?;
    let counter_value = match response {
        QueryResult::Error((msg, _)) => return Response::from_html(format!("Error: {}", msg)),
        QueryResult::Success((result, _)) => {
            let first_row = result
                .rows
                .first()
                .ok_or(worker::Error::from("No rows found in the counter table"))?;
            match first_row.cells.get("value") {
                Some(v) => match v {
                    CellValue::Number(v) => *v,
                    _ => return Response::from_html("Unexpected counter value"),
                },
                _ => return Response::from_html("No value for 'value' column"),
            }
        }
    };

    let update_result = db
        .transaction([Statement::with_params(
            "UPDATE counter SET value = ? WHERE key = ?",
            &[CellValue::Number(counter_value + 1), "turso".into()],
        )])
        .await;
    let counter_status = match update_result {
        Ok(_) => format!(
            "Counter was just successfully bumped: {} -> {}. Congrats!",
            counter_value,
            counter_value + 1,
        ),
        Err(e) => format!("Counter update error: {e}"),
    };

    let mut html =
        "And here's the whole database, dumped: <br /><table style=\"border: 1px solid\">"
            .to_string();
    let response = db.execute("SELECT * FROM counter").await?;
    match response {
        QueryResult::Error((msg, _)) => return Response::from_html(format!("Error: {}", msg)),
        QueryResult::Success((result, _)) => {
            for column in &result.columns {
                html += &format!("<th style=\"border: 1px solid\">{}</th>", column);
            }
            for row in result.rows {
                html += "<tr style=\"border: 1px solid\">";
                for column in &result.columns {
                    html += &format!("<td>{:?}</td>", row.cells[column]);
                }
                html += "</tr>";
            }
        }
    };
    html += "</table>";

    let html = counter_status;
    Response::from_html(html)
}

#[event(fetch)]
pub async fn main(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    log_request(&req);

    // Optionally, get more helpful error messages written to the console in the case of a panic.
    utils::set_panic_hook();

    // Optionally, use the Router to handle matching endpoints, use ":name" placeholders, or "*name"
    // catch-alls to match on specific patterns. Alternatively, use `Router::with_data(D)` to
    // provide arbitrary data that will be accessible in each route via the `ctx.data()` method.
    let router = Router::new();

    // Add as many routes as your Worker needs! Each route will get a `Request` for handling HTTP
    // functionality and a `RouteContext` which you can use to  and get route parameters and
    // Environment bindings like KV Stores, Durable Objects, Secrets, and Variables.
    router
        .get_async("/", |req, ctx| async move {
            let db = match libsql_client::Connection::connect_from_ctx(&ctx) {
                Ok(db) => db,
                Err(e) => {
                    console_log!("Error {e}");
                    return Response::from_html(format!("Error establishing connection: {e}"));
                }
            };
            serve(req, db).await
        })
        .get("/worker-version", |_, ctx| {
            let version = ctx.var("WORKERS_RS_VERSION")?.to_string();
            Response::ok(version)
        })
        .run(req, env)
        .await
}
