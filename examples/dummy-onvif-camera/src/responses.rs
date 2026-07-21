use crate::config::DeviceCfg;

const NS: &str = concat!(
    r#" xmlns:env="http://www.w3.org/2003/05/soap-envelope""#,
    r#" xmlns:tds="http://www.onvif.org/ver10/device/wsdl""#,
    r#" xmlns:trt="http://www.onvif.org/ver10/media/wsdl""#,
    r#" xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl""#,
    r#" xmlns:tt="http://www.onvif.org/ver10/schema""#,
    r#" xmlns:ter="http://www.onvif.org/ver10/error""#,
);

/// Wrap a body-inner XML fragment in a namespaced SOAP 1.2 envelope.
pub(crate) fn envelope(body_inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><env:Envelope{NS}><env:Body>{body_inner}</env:Body></env:Envelope>"#
    )
}

pub fn get_capabilities(service_url: &str) -> String {
    envelope(&format!(
        r#"<tds:GetCapabilitiesResponse><tds:Capabilities>
<tt:Media><tt:XAddr>{service_url}</tt:XAddr></tt:Media>
<tt:PTZ><tt:XAddr>{service_url}</tt:XAddr></tt:PTZ>
</tds:Capabilities></tds:GetCapabilitiesResponse>"#
    ))
}

pub fn device_information(cfg: &DeviceCfg) -> String {
    envelope(&format!(
        r#"<tds:GetDeviceInformationResponse>
<tds:Manufacturer>{m}</tds:Manufacturer>
<tds:Model>{mo}</tds:Model>
<tds:FirmwareVersion>{f}</tds:FirmwareVersion>
<tds:SerialNumber>{s}</tds:SerialNumber>
<tds:HardwareId>HW-1</tds:HardwareId>
</tds:GetDeviceInformationResponse>"#,
        m = cfg.manufacturer,
        mo = cfg.model,
        f = cfg.firmware,
        s = cfg.serial
    ))
}

pub fn get_profiles() -> String {
    envelope(
        r#"<trt:GetProfilesResponse><trt:Profiles token="Profile_1" fixed="true">
<tt:Name>Profile_1</tt:Name>
<tt:VideoEncoderConfiguration token="VEC_1">
<tt:Name>VEC_1</tt:Name><tt:Encoding>H264</tt:Encoding>
<tt:Resolution><tt:Width>1920</tt:Width><tt:Height>1080</tt:Height></tt:Resolution>
</tt:VideoEncoderConfiguration>
</trt:Profiles></trt:GetProfilesResponse>"#,
    )
}

pub fn get_stream_uri(rtsp_url: &str) -> String {
    envelope(&format!(
        r#"<trt:GetStreamUriResponse><trt:MediaUri>
<tt:Uri>{rtsp_url}</tt:Uri>
<tt:InvalidAfterConnect>false</tt:InvalidAfterConnect>
<tt:InvalidAfterReboot>false</tt:InvalidAfterReboot>
<tt:Timeout>PT60S</tt:Timeout>
</trt:MediaUri></trt:GetStreamUriResponse>"#
    ))
}

pub fn get_presets() -> String {
    envelope(
        r#"<tptz:GetPresetsResponse>
<tptz:Preset token="Preset_1"><tt:Name>Preset_1</tt:Name></tptz:Preset>
<tptz:Preset token="Preset_2"><tt:Name>Preset_2</tt:Name></tptz:Preset>
</tptz:GetPresetsResponse>"#,
    )
}

/// Empty ack for ContinuousMove/Stop/GotoPreset. Pass the response element name,
/// e.g. "tptz:ContinuousMoveResponse".
pub fn ptz_ack(op_response_element: &str) -> String {
    envelope(&format!(r#"<{op_response_element}/>"#))
}

/// Non-2xx body: a SOAP Fault whose Subcode Value contains "NotAuthorized",
/// which is exactly what onvif-rs classifies as an authorization failure.
pub fn fault_not_authorized() -> String {
    envelope(
        r#"<env:Fault><env:Code><env:Value>env:Sender</env:Value>
<env:Subcode><env:Value>ter:NotAuthorized</env:Value></env:Subcode></env:Code>
<env:Reason><env:Text xml:lang="en">Sender not authorized</env:Text></env:Reason>
</env:Fault>"#,
    )
}

pub fn fault_action_not_supported() -> String {
    envelope(
        r#"<env:Fault><env:Code><env:Value>env:Receiver</env:Value>
<env:Subcode><env:Value>ter:ActionNotSupported</env:Value></env:Subcode></env:Code>
<env:Reason><env:Text xml:lang="en">Action not supported</env:Text></env:Reason>
</env:Fault>"#,
    )
}

#[cfg(test)]
#[path = "responses_test.rs"]
mod responses_test;
