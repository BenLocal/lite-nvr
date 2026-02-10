use anyhow::Ok;
use turso::{Connection, Row};

#[derive(Debug, Default)]
pub struct Kv {
    pub id: i64,
    pub module: String,
    pub key: String,
    pub sub_key: Option<String>,
    pub value: Option<String>,
}

pub async fn by_id(id: i64, conn: &Connection) -> anyhow::Result<Option<Kv>> {
    let mut rows = conn
        .query(
            "SELECT id, module, key, sub_key, value FROM kvs WHERE id = ?1 LIMIT 1",
            (id,),
        )
        .await?;
    let row = rows.next().await?;
    row_to_kv(row)
}

pub async fn by_module(module: &str, conn: &Connection) -> anyhow::Result<Vec<Kv>> {
    let mut rows = conn
        .query(
            "SELECT id, module, key, sub_key, value FROM kvs WHERE module = ?1",
            (module,),
        )
        .await?;
    let mut kvs = Vec::new();
    while let Some(row) = rows.next().await? {
        if let Some(kv) = row_to_kv(Some(row))? {
            kvs.push(kv);
        }
    }
    Ok(kvs)
}

pub async fn by_module_and_key(
    module: &str,
    key: &str,
    conn: &Connection,
) -> anyhow::Result<Option<Kv>> {
    let mut rows = conn
        .query("SELECT id, module, key, sub_key, value FROM kvs WHERE module = ?1 AND key = ?2 limit 1", (module, key))
        .await?;
    row_to_kv(rows.next().await?)
}

pub async fn by_module_and_key_and_sub_key(
    module: &str,
    key: &str,
    sub_key: &str,
    conn: &Connection,
) -> anyhow::Result<Option<Kv>> {
    let mut rows = conn
        .query("SELECT id, module, key, sub_key, value FROM kvs WHERE module = ?1 AND key = ?2 AND sub_key = ?3", (module, key, sub_key))
        .await?;
    row_to_kv(rows.next().await?)
}

fn row_to_kv(row: Option<Row>) -> anyhow::Result<Option<Kv>> {
    if let Some(row) = row {
        let id = row
            .get_value(0)
            .map_err(anyhow::Error::from)?
            .as_integer()
            .ok_or_else(|| anyhow::anyhow!("id is null"))?
            .to_owned();
        let module = row
            .get_value(1)?
            .as_text()
            .ok_or_else(|| anyhow::anyhow!("module is null"))?
            .to_owned();
        let key = row
            .get_value(2)?
            .as_text()
            .ok_or_else(|| anyhow::anyhow!("key is null"))?
            .to_owned();
        let sub_key = row.get_value(3)?.as_text().map(|s| s.to_owned());
        let value = row.get_value(4)?.as_text().map(|s| s.to_owned());
        return Ok(Some(Kv {
            id,
            module,
            key,
            sub_key,
            value,
        }));
    }

    Ok(None)
}
