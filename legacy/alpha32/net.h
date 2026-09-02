#ifndef NET_H
#define NET_H

#include "types.h"

#define NET_MAX_INTERFACES 4
#define NET_MAX_CONNECTIONS 16

#define NET_UP         0x01
#define NET_LOOPBACK   0x02
#define NET_DHCP       0x04

#define NET_TCP 0x06
#define NET_UDP 0x11

#define NET_ESTABLISHED 1
#define NET_CLOSED      2
#define NET_LISTEN      3

typedef struct __attribute__((packed)) {
    char name[16];
    uint32_t ip;
    uint32_t mask;
    uint32_t flags;
    uint32_t mtu;
} net_interface_t;

typedef struct __attribute__((packed)) {
    uint32_t local_ip;
    uint32_t local_port;
    uint32_t remote_ip;
    uint32_t remote_port;
    uint32_t proto;
    uint32_t state;
    uint32_t owner_pid;
    uint32_t snap_id;
} net_connection_t;

int net_init(void);
int net_interface_add(const char *name, uint32_t ip, uint32_t mask, uint32_t mtu);
int net_interface_list(char *out, int out_len);
int net_connection_open(uint32_t local_ip, uint32_t local_port, uint32_t remote_ip, uint32_t remote_port, uint32_t proto);
int net_connection_close(int conn_id);
int net_connection_list(char *out, int out_len);
int net_loopback_send(const uint8_t *data, int len);
int net_loopback_recv(uint8_t *buf, int maxlen);
int net_history_list(char *out, int out_len);
void net_register_observers(void);
int net_get_interface_count(void);
int net_get_connection_count(void);

#endif
