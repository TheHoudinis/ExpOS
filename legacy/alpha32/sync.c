#include "types.h"
#include "sync.h"

void mutex_init(mutex_t *m) {
    m->locked = 0;
    m->owner = 0xFFFFFFFF;
}

void mutex_lock(mutex_t *m) {
    while (__sync_lock_test_and_set(&m->locked, 1)) {
        __asm__ volatile("pause");
    }
    m->owner = 1;
}

int mutex_trylock(mutex_t *m) {
    return __sync_lock_test_and_set(&m->locked, 1) == 0;
}

void mutex_unlock(mutex_t *m) {
    m->owner = 0xFFFFFFFF;
    __sync_lock_release(&m->locked);
}

void sem_init(semaphore_t *s, int val) {
    s->val = val;
    s->waiters = 0;
}

void sem_wait(semaphore_t *s) {
    while (1) {
        cli();
        if (s->val > 0) {
            s->val--;
            sti();
            return;
        }
        s->waiters++;
        sti();
        __asm__ volatile("pause");
    }
}

void sem_signal(semaphore_t *s) {
    cli();
    s->val++;
    s->waiters = 0;
    sti();
}
