use std::time::Duration;

use gb28181::{
    CatalogItem, GbClient, GbClientConfig, GbEvent, GbServer, GbServerConfig, PtzCommand,
};

const PLATFORM: &str = "34020000002000000001";
const DOMAIN: &str = "3402000000";
const DEVICE: &str = "34020000001110000001";
const CHANNEL: &str = "34020000001320000001";

#[tokio::test]
async fn device_control_reaches_device_with_correct_ptzcmd() {
    // platform
    let scfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    let (server, mut server_events) = GbServer::bind(scfg).await.unwrap();
    let server_addr = server.local_addr();

    // device (client role)
    let mut ccfg = GbClientConfig::new(DEVICE, DOMAIN, PLATFORM, server_addr);
    ccfg.listen = "127.0.0.1:0".parse().unwrap();
    ccfg.channels = vec![CatalogItem {
        device_id: CHANNEL.into(),
        name: "cam".into(),
        status: "ON".into(),
    }];
    let (client, mut client_events) = GbClient::bind(ccfg).await.unwrap();
    client.register().await.unwrap();

    // wait until the platform sees the device registered (so `dests` is populated)
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = server_events.recv().await {
            if matches!(e, GbEvent::Registered { .. }) {
                return;
            }
        }
    })
    .await
    .expect("device never registered");

    // send a PTZ "up" and assert the device receives the exact PTZCmd
    let cmd = PtzCommand::Move {
        up: true,
        tilt_speed: 0x20,
        down: false,
        left: false,
        right: false,
        zoom_in: false,
        zoom_out: false,
        pan_speed: 0,
        zoom_speed: 0,
    };
    server.device_control(DEVICE, CHANNEL, cmd).await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = client_events.recv().await {
            if let GbEvent::DeviceControlReceived { device_id, ptz_cmd } = e {
                return (device_id, ptz_cmd);
            }
        }
        (String::new(), String::new())
    })
    .await
    .expect("device never got the control");

    assert_eq!(got.0, CHANNEL); // <DeviceID> is the controlled channel
    assert_eq!(got.1, "A50F0108002000DD"); // up, tilt speed 0x20

    client.shutdown();
    server.shutdown();
}
