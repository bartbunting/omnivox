POWERSHELL ?= powershell.exe
BUILD_SCRIPT = $(shell wslpath -m "$(CURDIR)/build.ps1")

ifdef OMNIVOX_CSC
COMPILER_ARGS = \
	-CompilerPath "$(shell wslpath -m "$(OMNIVOX_CSC)")" \
	-ReferenceDirectory "$(shell wslpath -m "$(OMNIVOX_REFERENCE_DIR)")"
endif

all:
	$(POWERSHELL) -NoProfile -ExecutionPolicy Bypass -File "$(BUILD_SCRIPT)" \
		$(COMPILER_ARGS)
	chmod +x bin/OmnivoxEloquenceHelper32.exe \
		bin/OmnivoxDectalkHelper32.exe

eloquence:
	$(POWERSHELL) -NoProfile -ExecutionPolicy Bypass -File "$(BUILD_SCRIPT)" \
		$(COMPILER_ARGS) -Engine eloquence
	chmod +x bin/OmnivoxEloquenceHelper32.exe

dectalk:
	$(POWERSHELL) -NoProfile -ExecutionPolicy Bypass -File "$(BUILD_SCRIPT)" \
		$(COMPILER_ARGS) -Engine dectalk
	chmod +x bin/OmnivoxDectalkHelper32.exe

clean:
	$(POWERSHELL) -NoProfile -ExecutionPolicy Bypass -File "$(BUILD_SCRIPT)" \
		-Clean

.PHONY: all eloquence dectalk clean
