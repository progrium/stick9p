//! Shared socket buffers (static) to keep embassy task futures small.

use static_cell::StaticCell;

pub struct TcpBufs {
    pub rx: [u8; 1536],
    pub tx: [u8; 2048],
}

static TCP_9P: StaticCell<TcpBufs> = StaticCell::new();
static mut TCP_9P_PTR: *mut TcpBufs = core::ptr::null_mut();

static TCP_WS: StaticCell<TcpBufs> = StaticCell::new();
static mut TCP_WS_PTR: *mut TcpBufs = core::ptr::null_mut();

static WS_FRAME: StaticCell<[u8; 2048]> = StaticCell::new();
static mut WS_FRAME_PTR: *mut [u8; 2048] = core::ptr::null_mut();

// Link-time .bss — never call SessionStorage::new() on a task stack (~12 KiB each).
static mut NINEP_TCP_STORAGE: ninep::buffers::SessionStorage = ninep::buffers::SessionStorage::new();
static mut NINEP_WS_STORAGE: ninep::buffers::SessionStorage = ninep::buffers::SessionStorage::new();

static TCP_HTTP0: StaticCell<TcpBufs> = StaticCell::new();
static mut TCP_HTTP0_PTR: *mut TcpBufs = core::ptr::null_mut();

static TCP_HTTP1: StaticCell<TcpBufs> = StaticCell::new();
static mut TCP_HTTP1_PTR: *mut TcpBufs = core::ptr::null_mut();

pub fn tcp_9p() -> &'static mut TcpBufs {
    unsafe {
        if TCP_9P_PTR.is_null() {
            TCP_9P_PTR = TCP_9P.init(TcpBufs {
                rx: [0; 1536],
                tx: [0; 2048],
            });
        }
        &mut *TCP_9P_PTR
    }
}

pub fn tcp_ws() -> &'static mut TcpBufs {
    unsafe {
        if TCP_WS_PTR.is_null() {
            TCP_WS_PTR = TCP_WS.init(TcpBufs {
                rx: [0; 1536],
                tx: [0; 2048],
            });
        }
        &mut *TCP_WS_PTR
    }
}

pub fn ws_frame() -> &'static mut [u8; 2048] {
    unsafe {
        if WS_FRAME_PTR.is_null() {
            WS_FRAME_PTR = WS_FRAME.init([0; 2048]);
        }
        &mut *WS_FRAME_PTR
    }
}

pub fn ninep_tcp_storage() -> &'static mut ninep::buffers::SessionStorage {
    unsafe { &mut *core::ptr::addr_of_mut!(NINEP_TCP_STORAGE) }
}

pub fn ninep_ws_storage() -> &'static mut ninep::buffers::SessionStorage {
    unsafe { &mut *core::ptr::addr_of_mut!(NINEP_WS_STORAGE) }
}

pub fn tcp_http(slot: usize) -> &'static mut TcpBufs {
    unsafe {
        match slot {
            0 => {
                if TCP_HTTP0_PTR.is_null() {
                    TCP_HTTP0_PTR = TCP_HTTP0.init(TcpBufs {
                        rx: [0; 1536],
                        tx: [0; 2048],
                    });
                }
                &mut *TCP_HTTP0_PTR
            }
            1 => {
                if TCP_HTTP1_PTR.is_null() {
                    TCP_HTTP1_PTR = TCP_HTTP1.init(TcpBufs {
                        rx: [0; 1536],
                        tx: [0; 2048],
                    });
                }
                &mut *TCP_HTTP1_PTR
            }
            _ => panic!("invalid http slot"),
        }
    }
}

static DHCP_BUF: StaticCell<[u8; 512]> = StaticCell::new();
static mut DHCP_BUF_PTR: *mut [u8; 512] = core::ptr::null_mut();

static DHCP_GW: StaticCell<[core::net::Ipv4Addr; 1]> = StaticCell::new();
static mut DHCP_GW_PTR: *mut [core::net::Ipv4Addr; 1] = core::ptr::null_mut();

static EDGE_UDP: StaticCell<edge_nal_embassy::UdpBuffers<2, 512, 512, 8>> = StaticCell::new();
static mut EDGE_UDP_PTR: *mut edge_nal_embassy::UdpBuffers<2, 512, 512, 8> = core::ptr::null_mut();

pub fn dhcp_packet() -> &'static mut [u8; 512] {
    unsafe {
        if DHCP_BUF_PTR.is_null() {
            DHCP_BUF_PTR = DHCP_BUF.init([0; 512]);
        }
        &mut *DHCP_BUF_PTR
    }
}

pub fn dhcp_gw() -> &'static mut [core::net::Ipv4Addr; 1] {
    unsafe {
        if DHCP_GW_PTR.is_null() {
            DHCP_GW_PTR = DHCP_GW.init([core::net::Ipv4Addr::UNSPECIFIED]);
        }
        &mut *DHCP_GW_PTR
    }
}

pub fn edge_udp_buffers() -> &'static edge_nal_embassy::UdpBuffers<2, 512, 512, 8> {
    unsafe {
        if EDGE_UDP_PTR.is_null() {
            EDGE_UDP_PTR = EDGE_UDP.init(edge_nal_embassy::UdpBuffers::new());
        }
        &*EDGE_UDP_PTR
    }
}

static DNS_RX: StaticCell<[u8; 512]> = StaticCell::new();
static mut DNS_RX_PTR: *mut [u8; 512] = core::ptr::null_mut();

static DNS_TX: StaticCell<[u8; 512]> = StaticCell::new();
static mut DNS_TX_PTR: *mut [u8; 512] = core::ptr::null_mut();

pub fn dns_rx() -> &'static mut [u8; 512] {
    unsafe {
        if DNS_RX_PTR.is_null() {
            DNS_RX_PTR = DNS_RX.init([0; 512]);
        }
        &mut *DNS_RX_PTR
    }
}

pub fn dns_tx() -> &'static mut [u8; 512] {
    unsafe {
        if DNS_TX_PTR.is_null() {
            DNS_TX_PTR = DNS_TX.init([0; 512]);
        }
        &mut *DNS_TX_PTR
    }
}

static HTTP_REQ0: StaticCell<[u8; 1024]> = StaticCell::new();
static mut HTTP_REQ0_PTR: *mut [u8; 1024] = core::ptr::null_mut();

static HTTP_REQ1: StaticCell<[u8; 1024]> = StaticCell::new();
static mut HTTP_REQ1_PTR: *mut [u8; 1024] = core::ptr::null_mut();

pub fn http_req(slot: usize) -> &'static mut [u8; 1024] {
    unsafe {
        match slot {
            0 => {
                if HTTP_REQ0_PTR.is_null() {
                    HTTP_REQ0_PTR = HTTP_REQ0.init([0; 1024]);
                }
                &mut *HTTP_REQ0_PTR
            }
            1 => {
                if HTTP_REQ1_PTR.is_null() {
                    HTTP_REQ1_PTR = HTTP_REQ1.init([0; 1024]);
                }
                &mut *HTTP_REQ1_PTR
            }
            _ => panic!("invalid http req slot"),
        }
    }
}

static HTTP_RES: StaticCell<[u8; 2048]> = StaticCell::new();
static mut HTTP_RES_PTR: *mut [u8; 2048] = core::ptr::null_mut();

pub fn http_res() -> &'static mut [u8; 2048] {
    unsafe {
        if HTTP_RES_PTR.is_null() {
            HTTP_RES_PTR = HTTP_RES.init([0; 2048]);
        }
        &mut *HTTP_RES_PTR
    }
}
