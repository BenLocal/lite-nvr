//! Typed MANSCDP messages + lenient decode / strict encode.
//!
//! Decode path: `decode_xml` (charset) -> quick-xml serde into a typed message.
//! Unknown elements are ignored; missing optionals tolerated.

use serde::Deserialize;

use crate::encoding::decode_xml;
use crate::error::{GbError, Result};

/// The MANSCDP command discriminator (`<CmdType>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CmdType {
    Keepalive,
    Catalog,
    DeviceInfo,
    Other(String),
}

impl CmdType {
    fn from_str(s: &str) -> Self {
        match s {
            "Keepalive" => CmdType::Keepalive,
            "Catalog" => CmdType::Catalog,
            "DeviceInfo" => CmdType::DeviceInfo,
            other => CmdType::Other(other.to_string()),
        }
    }
}

/// A decoded inbound Keepalive `<Notify>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keepalive {
    pub sn: u64,
    pub device_id: String,
    pub status: String,
}

// --- raw serde shapes (lenient) ---

#[derive(Debug, Deserialize)]
struct RawNotify {
    #[serde(rename = "CmdType", default)]
    cmd_type: String,
    #[serde(rename = "SN", default)]
    sn: u64,
    #[serde(rename = "DeviceID", default)]
    device_id: String,
    #[serde(rename = "Status", default)]
    status: String,
}

/// Peek the `<CmdType>` of any MANSCDP body without committing to a full type.
pub fn peek_cmd_type(body: &[u8]) -> Result<CmdType> {
    let xml = decode_xml(body)?;
    let raw: RawNotify =
        quick_xml::de::from_str(&xml).map_err(|e| GbError::XmlDecode(e.to_string()))?;
    Ok(CmdType::from_str(&raw.cmd_type))
}

pub fn decode_keepalive(body: &[u8]) -> Result<Keepalive> {
    let xml = decode_xml(body)?;
    let raw: RawNotify =
        quick_xml::de::from_str(&xml).map_err(|e| GbError::XmlDecode(e.to_string()))?;
    Ok(Keepalive {
        sn: raw.sn,
        device_id: raw.device_id,
        status: raw.status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEEPALIVE: &[u8] = br#"<?xml version="1.0" encoding="GB2312"?>
<Notify>
<CmdType>Keepalive</CmdType>
<SN>7</SN>
<DeviceID>34020000001320000001</DeviceID>
<Status>OK</Status>
</Notify>"#;

    #[test]
    fn peek_returns_keepalive() {
        assert_eq!(peek_cmd_type(KEEPALIVE).unwrap(), CmdType::Keepalive);
    }

    #[test]
    fn decodes_keepalive_fields() {
        let k = decode_keepalive(KEEPALIVE).unwrap();
        assert_eq!(k.sn, 7);
        assert_eq!(k.device_id, "34020000001320000001");
        assert_eq!(k.status, "OK");
    }

    #[test]
    fn tolerates_missing_status() {
        let body =
            br#"<Notify><CmdType>Keepalive</CmdType><SN>1</SN><DeviceID>d</DeviceID></Notify>"#;
        let k = decode_keepalive(body).unwrap();
        assert_eq!(k.status, ""); // defaulted, not an error
    }

    #[test]
    fn peek_returns_other_for_unknown_cmd_type() {
        // `decode_xml` does not change case — "Subscribe" must be preserved exactly.
        let body = br#"<Notify>
<CmdType>Subscribe</CmdType>
<SN>1</SN>
<DeviceID>34020000001320000001</DeviceID>
</Notify>"#;
        assert_eq!(
            peek_cmd_type(body).unwrap(),
            CmdType::Other("Subscribe".into())
        );
    }
}

// --- Task 6: Catalog query encode + response decode ---

/// A Catalog query `<Query>` we SEND to a device. Strict UTF-8 encode.
///
/// # Safety / injection contract
/// `device_id` MUST be a plain GB code (digits only, no XML metacharacters).
/// It is interpolated directly into the XML without escaping.
pub fn encode_catalog_query(sn: u64, device_id: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<Query>\r\n<CmdType>Catalog</CmdType>\r\n<SN>{sn}</SN>\r\n\
<DeviceID>{device_id}</DeviceID>\r\n</Query>\r\n"
    )
}

/// One channel item from a Catalog response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogItem {
    pub device_id: String,
    pub name: String,
    pub status: String,
}

/// A decoded Catalog response chunk (`SumNum` is the total across all chunks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogResponse {
    pub sn: u64,
    pub device_id: String,
    pub sum_num: u32,
    pub items: Vec<CatalogItem>,
}

#[derive(Debug, Deserialize)]
struct RawCatalogResponse {
    #[serde(rename = "SN", default)]
    sn: u64,
    #[serde(rename = "DeviceID", default)]
    device_id: String,
    #[serde(rename = "SumNum", default)]
    sum_num: u32,
    #[serde(rename = "DeviceList", default)]
    device_list: RawDeviceList,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeviceList {
    #[serde(rename = "Item", default)]
    items: Vec<RawCatalogItem>,
}

#[derive(Debug, Deserialize)]
struct RawCatalogItem {
    #[serde(rename = "DeviceID", default)]
    device_id: String,
    #[serde(rename = "Name", default)]
    name: String,
    #[serde(rename = "Status", default)]
    status: String,
}

pub fn decode_catalog_response(body: &[u8]) -> Result<CatalogResponse> {
    let xml = decode_xml(body)?;
    let raw: RawCatalogResponse =
        quick_xml::de::from_str(&xml).map_err(|e| GbError::XmlDecode(e.to_string()))?;
    Ok(CatalogResponse {
        sn: raw.sn,
        device_id: raw.device_id,
        sum_num: raw.sum_num,
        items: raw
            .device_list
            .items
            .into_iter()
            .map(|i| CatalogItem {
                device_id: i.device_id,
                name: i.name,
                status: i.status,
            })
            .collect(),
    })
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[test]
    fn query_is_well_formed() {
        let q = encode_catalog_query(42, "34020000002000000001");
        assert!(q.contains("<CmdType>Catalog</CmdType>"));
        assert!(q.contains("<SN>42</SN>"));
        assert!(q.contains("<DeviceID>34020000002000000001</DeviceID>"));
    }

    #[test]
    fn decodes_multi_item_response() {
        let body = br#"<?xml version="1.0" encoding="GB2312"?>
<Response>
<CmdType>Catalog</CmdType>
<SN>42</SN>
<DeviceID>34020000002000000001</DeviceID>
<SumNum>2</SumNum>
<DeviceList Num="2">
<Item><DeviceID>34020000001320000001</DeviceID><Name>door</Name><Status>ON</Status></Item>
<Item><DeviceID>34020000001320000002</DeviceID><Name>yard</Name><Status>ON</Status></Item>
</DeviceList>
</Response>"#;
        let r = decode_catalog_response(body).unwrap();
        assert_eq!(r.sn, 42);
        assert_eq!(r.sum_num, 2);
        assert_eq!(r.items.len(), 2);
        assert_eq!(r.items[0].device_id, "34020000001320000001");
        assert_eq!(r.items[1].name, "yard");
    }

    #[test]
    fn tolerates_empty_device_list() {
        let body = br#"<Response><CmdType>Catalog</CmdType><SN>1</SN><DeviceID>d</DeviceID><SumNum>0</SumNum></Response>"#;
        let r = decode_catalog_response(body).unwrap();
        assert_eq!(r.items.len(), 0);
        assert_eq!(r.sum_num, 0);
    }
}

// --- Task 7: Catalog aggregator ---

/// Accumulates Catalog response chunks for one `SN`. Pure state machine — the
/// caller feeds decoded chunks and decides when to stop (timeout lives elsewhere).
#[derive(Debug)]
pub struct CatalogAccumulator {
    sn: u64,
    sum_num: u32,
    items: Vec<CatalogItem>,
}

impl CatalogAccumulator {
    pub fn new(sn: u64) -> Self {
        Self {
            sn,
            sum_num: 0,
            items: Vec::new(),
        }
    }

    /// Feed a decoded chunk. Chunks with a mismatched SN are ignored (return false).
    pub fn push(&mut self, chunk: CatalogResponse) -> bool {
        if chunk.sn != self.sn {
            return false;
        }
        if chunk.sum_num > self.sum_num {
            self.sum_num = chunk.sum_num;
        }
        self.items.extend(chunk.items);
        true
    }

    pub fn is_complete(&self) -> bool {
        self.sum_num > 0 && self.items.len() as u32 >= self.sum_num
    }

    /// Consume into the final result. `incomplete` is true if fewer than
    /// `sum_num` items were collected (partial-tolerant per the resilience policy).
    pub fn finish(self) -> Catalog {
        let incomplete = self.sum_num > 0 && (self.items.len() as u32) < self.sum_num;
        Catalog {
            sum_num: self.sum_num,
            items: self.items,
            incomplete,
        }
    }
}

/// The aggregated catalog returned to callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub sum_num: u32,
    pub items: Vec<CatalogItem>,
    pub incomplete: bool,
}

#[cfg(test)]
mod accumulator_tests {
    use super::*;

    fn item(id: &str) -> CatalogItem {
        CatalogItem {
            device_id: id.into(),
            name: id.into(),
            status: "ON".into(),
        }
    }

    fn chunk(sn: u64, sum: u32, ids: &[&str]) -> CatalogResponse {
        CatalogResponse {
            sn,
            device_id: "d".into(),
            sum_num: sum,
            items: ids.iter().map(|i| item(i)).collect(),
        }
    }

    #[test]
    fn aggregates_two_chunks_to_complete() {
        let mut acc = CatalogAccumulator::new(42);
        assert!(acc.push(chunk(42, 3, &["a", "b"])));
        assert!(!acc.is_complete());
        assert!(acc.push(chunk(42, 3, &["c"])));
        assert!(acc.is_complete());
        let cat = acc.finish();
        assert_eq!(cat.items.len(), 3);
        assert!(!cat.incomplete);
    }

    #[test]
    fn ignores_mismatched_sn() {
        let mut acc = CatalogAccumulator::new(42);
        assert!(!acc.push(chunk(99, 1, &["x"])));
        assert_eq!(acc.finish().items.len(), 0);
    }

    #[test]
    fn partial_finish_flags_incomplete() {
        let mut acc = CatalogAccumulator::new(1);
        acc.push(chunk(1, 5, &["a", "b"]));
        let cat = acc.finish(); // only 2 of 5 arrived
        assert!(cat.incomplete);
        assert_eq!(cat.items.len(), 2);
    }
}

// --- Task 8: DeviceInfo query encode + response decode ---

/// Build a DeviceInfo query `<Query>` we SEND to a device. Strict UTF-8 encode.
///
/// # Safety / injection contract
/// `device_id` MUST be a plain GB code (digits only, no XML metacharacters).
/// It is interpolated directly into the XML without escaping.
pub fn encode_deviceinfo_query(sn: u64, device_id: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<Query>\r\n<CmdType>DeviceInfo</CmdType>\r\n<SN>{sn}</SN>\r\n\
<DeviceID>{device_id}</DeviceID>\r\n</Query>\r\n"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub sn: u64,
    pub device_id: String,
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub channel: u32,
}

#[derive(Debug, Deserialize)]
struct RawDeviceInfo {
    #[serde(rename = "SN", default)]
    sn: u64,
    #[serde(rename = "DeviceID", default)]
    device_id: String,
    #[serde(rename = "Manufacturer", default)]
    manufacturer: String,
    #[serde(rename = "Model", default)]
    model: String,
    #[serde(rename = "Firmware", default)]
    firmware: String,
    #[serde(rename = "Channel", default)]
    channel: u32,
}

pub fn decode_deviceinfo(body: &[u8]) -> Result<DeviceInfo> {
    let xml = decode_xml(body)?;
    let raw: RawDeviceInfo =
        quick_xml::de::from_str(&xml).map_err(|e| GbError::XmlDecode(e.to_string()))?;
    Ok(DeviceInfo {
        sn: raw.sn,
        device_id: raw.device_id,
        manufacturer: raw.manufacturer,
        model: raw.model,
        firmware: raw.firmware,
        channel: raw.channel,
    })
}

#[cfg(test)]
mod deviceinfo_tests {
    use super::*;

    #[test]
    fn query_well_formed() {
        let q = encode_deviceinfo_query(5, "34020000002000000001");
        assert!(q.contains("<CmdType>DeviceInfo</CmdType>"));
        assert!(q.contains("<SN>5</SN>"));
    }

    #[test]
    fn decodes_response() {
        let body = br#"<Response><CmdType>DeviceInfo</CmdType><SN>5</SN>
<DeviceID>34020000002000000001</DeviceID><Manufacturer>ACME</Manufacturer>
<Model>X1</Model><Firmware>1.2</Firmware><Channel>4</Channel></Response>"#;
        let d = decode_deviceinfo(body).unwrap();
        assert_eq!(d.manufacturer, "ACME");
        assert_eq!(d.model, "X1");
        assert_eq!(d.channel, 4);
    }
}

// --- P1-2: client-role encode + generic SN/device peek ---

/// Escape XML text content (NOT attribute-safe; we only emit text nodes).
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Keepalive `<Notify>` a device SENDS to its platform. Strict UTF-8 encode.
///
/// # Safety / injection contract
/// `device_id` MUST be a plain GB code (digits only, no XML metacharacters).
pub fn encode_keepalive_notify(sn: u64, device_id: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<Notify>\r\n<CmdType>Keepalive</CmdType>\r\n<SN>{sn}</SN>\r\n\
<DeviceID>{device_id}</DeviceID>\r\n<Status>OK</Status>\r\n</Notify>\r\n"
    )
}

/// Catalog `<Response>` a device SENDS answering a Catalog query.
/// Single chunk: `SumNum` == `items.len()`. Names/statuses are XML-escaped.
///
/// # Safety / injection contract
/// `device_id` and each `item.device_id` MUST be plain GB codes (digits only).
pub fn encode_catalog_response(sn: u64, device_id: &str, items: &[CatalogItem]) -> String {
    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<Response>\r\n<CmdType>Catalog</CmdType>\r\n");
    s.push_str(&format!(
        "<SN>{sn}</SN>\r\n<DeviceID>{device_id}</DeviceID>\r\n"
    ));
    s.push_str(&format!(
        "<SumNum>{}</SumNum>\r\n<DeviceList Num=\"{}\">\r\n",
        items.len(),
        items.len()
    ));
    for it in items {
        s.push_str(&format!(
            "<Item>\r\n<DeviceID>{}</DeviceID>\r\n<Name>{}</Name>\r\n<Status>{}</Status>\r\n</Item>\r\n",
            it.device_id,
            xml_escape(&it.name),
            xml_escape(&it.status)
        ));
    }
    s.push_str("</DeviceList>\r\n</Response>\r\n");
    s
}

/// Extract just `(SN, DeviceID)` from any MANSCDP body (query routing).
pub fn decode_sn_device(body: &[u8]) -> Result<(u64, String)> {
    let xml = decode_xml(body)?;
    let raw: RawNotify =
        quick_xml::de::from_str(&xml).map_err(|e| GbError::XmlDecode(e.to_string()))?;
    Ok((raw.sn, raw.device_id))
}

#[cfg(test)]
mod p1_2_encode_tests {
    use super::*;

    #[test]
    fn keepalive_notify_round_trips() {
        let body = encode_keepalive_notify(9, "34020000001320000001");
        let k = decode_keepalive(body.as_bytes()).unwrap();
        assert_eq!(k.sn, 9);
        assert_eq!(k.device_id, "34020000001320000001");
        assert_eq!(k.status, "OK");
        assert_eq!(peek_cmd_type(body.as_bytes()).unwrap(), CmdType::Keepalive);
    }

    #[test]
    fn catalog_response_round_trips() {
        let items = vec![
            CatalogItem {
                device_id: "34020000001320000001".into(),
                name: "door".into(),
                status: "ON".into(),
            },
            CatalogItem {
                device_id: "34020000001320000002".into(),
                name: "yard".into(),
                status: "ON".into(),
            },
        ];
        let body = encode_catalog_response(7, "34020000001110000001", &items);
        let r = decode_catalog_response(body.as_bytes()).unwrap();
        assert_eq!(r.sn, 7);
        assert_eq!(r.sum_num, 2);
        assert_eq!(r.items, items);
    }

    #[test]
    fn catalog_response_escapes_names() {
        let items = vec![CatalogItem {
            device_id: "34020000001320000001".into(),
            name: "a<b&c".into(),
            status: "ON".into(),
        }];
        let body = encode_catalog_response(1, "d", &items);
        assert!(body.contains("<Name>a&lt;b&amp;c</Name>"));
        let r = decode_catalog_response(body.as_bytes()).unwrap();
        assert_eq!(r.items[0].name, "a<b&c"); // unescaped on decode
    }

    #[test]
    fn sn_device_peek_works_on_any_root() {
        let q = encode_catalog_query(42, "34020000001110000001");
        let (sn, dev) = decode_sn_device(q.as_bytes()).unwrap();
        assert_eq!(sn, 42);
        assert_eq!(dev, "34020000001110000001");
    }
}
