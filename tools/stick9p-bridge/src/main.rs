//! Host bridge for `mount -t 9p` (Stage 4 in DESIGN.md).

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "stick9p-bridge", about = "Bridge StickS3 serial/BLE to TCP port 564")]
struct Args {
    /// Serial device (e.g. /dev/cu.usbserial-*)
    #[arg(short, long)]
    port: Option<String>,

    /// TCP listen address
    #[arg(long, default_value = "127.0.0.1:564")]
    listen: String,
}

fn main() {
    let args = Args::parse();
    eprintln!(
        "stick9p-bridge: not implemented yet (port={:?}, listen={})",
        args.port, args.listen
    );
    std::process::exit(1);
}
