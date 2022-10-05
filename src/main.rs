use read_jeelink::sync::SerialListener;

static DEVICE: &str = "/dev/tty.usbserial-AL006PX8";

fn main() -> std::io::Result<()> {
    println!("Open port on device");
    let listener = SerialListener::bind(DEVICE)?;
    println!("Ready to read");
    for frame in listener.incomming() {
        let frame = frame?;
        println!(
            "Sensor {:2}: Temperatur {:4}, Humidity {:2}, weak battery: {}, new battery: {}",
            frame.id, frame.temperature, frame.humidity, frame.weak_battery, frame.new_battery
        );
    }
    Ok(())
}
