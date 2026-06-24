//! Binario del daemon de notificaciones de escritorio de tawasuyu.

fn main() {
    pata_notify::init_tracing();
    pata_notify::run();
}
