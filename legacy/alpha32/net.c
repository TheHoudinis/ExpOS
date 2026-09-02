#include "types.h"
#include "net.h"
#include "hexafs.h"
#include "hexafs_disk.h"
#include "process.h"
#include "log.h"
#include "kobserve.h"

extern void *memset(void *dest, int c, size_t len);
extern void *memcpy(void *dest, const void *src, size_t len);
extern void print_string(const char *str);
extern void itoa(int num, char *str, int base);
extern size_t strlen(const char *str);

static net_interface_t net_interfaces[NET_MAX_INTERFACES];
static int net_if_count = 0;

static net_connection_t net_connections[NET_MAX_CONNECTIONS];
static int net_conn_count = 0;

static uint8_t loopback_ring[4096];
static int loopback_head = 0;
static int loopback_tail = 0;

static uint32_t net_history_count = 0;

int net_init(void) {
    memset(net_interfaces, 0, sizeof(net_interfaces));
    memset(net_connections, 0, sizeof(net_connections));
    net_if_count = 0;
    net_conn_count = 0;
    loopback_head = 0;
    loopback_tail = 0;
    net_history_count = 0;

    net_interface_t lo;
    memset(&lo, 0, sizeof(lo));
    int i = 0;
    const char *lon = "lo";
    while (lon[i] && i < 15) { lo.name[i] = lon[i]; i++; }
    lo.name[i] = 0;
    lo.ip = 0x7F000001;
    lo.mask = 0xFF000000;
    lo.flags = NET_UP | NET_LOOPBACK;
    lo.mtu = 1500;
    net_interfaces[net_if_count++] = lo;

    log_write(LOG_LEVEL_INFO, "net: loopback initialized");
    return 0;
}

int net_interface_add(const char *name, uint32_t ip, uint32_t mask, uint32_t mtu) {
    if (net_if_count >= NET_MAX_INTERFACES) return -1;
    net_interface_t *iface = &net_interfaces[net_if_count];
    int i = 0;
    while (name[i] && i < 15) { iface->name[i] = name[i]; i++; }
    iface->name[i] = 0;
    iface->ip = ip;
    iface->mask = mask;
    iface->flags = NET_UP;
    iface->mtu = mtu;
    net_if_count++;
    return net_if_count - 1;
}

int net_interface_list(char *out, int out_len) {
    int pos = 0;
    char buf[16];
    for (int i = 0; i < net_if_count; i++) {
        net_interface_t *iface = &net_interfaces[i];
        for (int j = 0; iface->name[j] && pos < out_len - 1; j++) out[pos++] = iface->name[j];
        out[pos++] = ' ';
        uint32_t ip = iface->ip;
        itoa(ip & 0xFF, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = '.';
        itoa((ip >> 8) & 0xFF, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = '.';
        itoa((ip >> 16) & 0xFF, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = '.';
        itoa((ip >> 24) & 0xFF, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        itoa((int)iface->mtu, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        out[pos++] = iface->flags & NET_UP ? 'U' : 'D';
        out[pos++] = iface->flags & NET_LOOPBACK ? 'L' : ' ';
        out[pos++] = '\n';
    }
    out[pos] = 0;
    return pos;
}

int net_connection_open(uint32_t local_ip, uint32_t local_port, uint32_t remote_ip, uint32_t remote_port, uint32_t proto) {
    if (net_conn_count >= NET_MAX_CONNECTIONS) return -1;
    net_connection_t *conn = &net_connections[net_conn_count];
    conn->local_ip = local_ip;
    conn->local_port = local_port;
    conn->remote_ip = remote_ip;
    conn->remote_port = remote_port;
    conn->proto = proto;
    conn->state = NET_ESTABLISHED;
    conn->owner_pid = (uint32_t)(current_task >= 0 ? tasks[current_task].pid : 0);
    conn->snap_id = 0;
    net_conn_count++;
    net_history_count++;
    return net_conn_count - 1;
}

int net_connection_close(int conn_id) {
    if (conn_id < 0 || conn_id >= net_conn_count) return -1;
    net_connections[conn_id].state = NET_CLOSED;
    return 0;
}

int net_connection_list(char *out, int out_len) {
    int pos = 0;
    char buf[16];
    const char *hdr = "PROTO LOCAL REMOTE STATE\n---- ----- ------ -----\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    for (int i = 0; i < net_conn_count; i++) {
        net_connection_t *c = &net_connections[i];
        if (c->state == NET_CLOSED) continue;
        out[pos++] = c->proto == NET_TCP ? 'T' : 'U';
        out[pos++] = ' ';
        itoa(c->local_ip & 0xFF, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ':';
        itoa((int)c->local_port, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        itoa(c->remote_ip & 0xFF, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ':';
        itoa((int)c->remote_port, buf, 10);
        for (int j = 0; buf[j] && pos < out_len - 1; j++) out[pos++] = buf[j];
        out[pos++] = ' ';
        const char *state_str = c->state == NET_ESTABLISHED ? "EST" : (c->state == NET_LISTEN ? "LST" : "CLS");
        for (int j = 0; state_str[j] && pos < out_len - 1; j++) out[pos++] = state_str[j];
        out[pos++] = '\n';
    }
    out[pos] = 0;
    return pos;
}

int net_loopback_send(const uint8_t *data, int len) {
    if (len > 4096) return -1;
    for (int i = 0; i < len; i++) {
        loopback_ring[loopback_head] = data[i];
        loopback_head = (loopback_head + 1) % 4096;
        if (loopback_head == loopback_tail) loopback_tail = (loopback_tail + 1) % 4096;
    }
    return len;
}

int net_loopback_recv(uint8_t *buf, int maxlen) {
    int total = 0;
    while (total < maxlen && loopback_tail != loopback_head) {
        buf[total++] = loopback_ring[loopback_tail];
        loopback_tail = (loopback_tail + 1) % 4096;
    }
    return total;
}

int net_history_list(char *out, int out_len) {
    int pos = 0;
    char buf[16];
    const char *hdr = "Network History:\n";
    for (int i = 0; hdr[i] && pos < out_len - 1; i++) out[pos++] = hdr[i];
    itoa((int)net_history_count, buf, 10);
    for (int i = 0; buf[i] && pos < out_len - 1; i++) out[pos++] = buf[i];
    const char *msg = " state changes recorded\n";
    for (int i = 0; msg[i] && pos < out_len - 1; i++) out[pos++] = msg[i];
    out[pos] = 0;
    return pos;
}

static int kobserve_net_ifaces(uint32_t filter, char *out, int out_len) {
    (void)filter;
    return net_interface_list(out, out_len);
}

static int kobserve_net_conns(uint32_t filter, char *out, int out_len) {
    (void)filter;
    return net_connection_list(out, out_len);
}

static int kobserve_net_history(uint32_t filter, char *out, int out_len) {
    (void)filter;
    return net_history_list(out, out_len);
}

void net_register_observers(void) {
    kobserve_register("/@net/interfaces", kobserve_net_ifaces, 0);
    kobserve_register("/@net/connections", kobserve_net_conns, 0);
    kobserve_register("/@net/history", kobserve_net_history, 0);
}

int net_get_interface_count(void) {
    return net_if_count;
}

int net_get_connection_count(void) {
    return net_conn_count;
}
