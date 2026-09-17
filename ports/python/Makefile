BUILD ?= ../../build/python
include ../../vendor/micropython/py/mkenv.mk
QSTR_DEFS = qstrdefsport.h
MICROPY_ROM_TEXT_COMPRESSION = 0
include $(TOP)/py/py.mk
INC = -I. -I$(TOP) -I$(BUILD)
CFLAGS = $(INC) -Os -std=gnu99 -DNDEBUG -ffreestanding -fno-builtin -fno-pie -fno-stack-protector -mno-red-zone -mno-sse -mno-sse2 -mno-mmx -msoft-float -ffunction-sections -fdata-sections -Wall -Wextra -Wno-unused-parameter
CSUPEROPT = -Os
SRC_C = runtime.c shared/runtime/gchelper_generic.c shared/libc/printf.c shared/libc/string0.c
SRC_QSTR += runtime.c
OBJ += $(PY_CORE_O) $(addprefix $(BUILD)/,$(SRC_C:.c=.o))
$(OBJ): Makefile mpconfigport.h
all: $(BUILD)/libexpos_python.a
$(BUILD)/libexpos_python.a: $(OBJ)
	$(AR) rcs $@ $^
include $(TOP)/py/mkrules.mk
