#ifndef SYNC_H
#define SYNC_H

#include "types.h"

typedef struct {
    volatile int locked;
    volatile uint32_t owner;
} mutex_t;

#define MUTEX_INIT {0, 0}

void mutex_init(mutex_t *m);
void mutex_lock(mutex_t *m);
int mutex_trylock(mutex_t *m);
void mutex_unlock(mutex_t *m);

typedef struct {
    volatile int val;
    volatile int waiters;
} semaphore_t;

void sem_init(semaphore_t *s, int val);
void sem_wait(semaphore_t *s);
void sem_signal(semaphore_t *s);

#endif
