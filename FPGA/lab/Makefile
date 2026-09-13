SHELL := /usr/bin/env bash
LAB_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
VENV_DIR ?= $(HOME)/.venvs/saturn-fpga
VENV_PYTHON := $(VENV_DIR)/bin/python
VIVADO ?= $(LAB_DIR)/scripts/vivado-windows.sh

.PHONY: doctor python-env python-test telemetry-test lint lint-strict formal check vivado-validate vivado-migrate-cdc vivado-migrate-iqmod vivado-migrate-telemetry vivado-build vivado-netlist-audit vivado-cdc export-prom sim-ddc sim-duc sim-iqmod sim-all

doctor:
	$(LAB_DIR)/scripts/doctor.sh

python-env:
	$(LAB_DIR)/scripts/bootstrap-python.sh

python-test:
	@test -x "$(VENV_PYTHON)" || { echo 'Run make python-env first.' >&2; exit 1; }
	cd $(LAB_DIR) && env -u PYTHONHOME -u PYTHONEXECUTABLE -u PYTHONNOUSERSITE PYTHONPATH=python $(VENV_PYTHON) -m unittest discover -s python/tests -v

telemetry-test:
	$(LAB_DIR)/scripts/telemetry-test.sh

lint:
	$(LAB_DIR)/scripts/lint.sh

lint-strict:
	SATURN_LINT_STRICT=1 $(LAB_DIR)/scripts/lint.sh

formal:
	$(LAB_DIR)/scripts/formal.sh

check: lint formal python-test telemetry-test

vivado-validate:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/validate.tcl

vivado-migrate-cdc:
	SATURN_VIVADO_WRITEBACK=1 $(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/migrate-cdc.tcl

vivado-migrate-iqmod:
	SATURN_VIVADO_WRITEBACK=1 $(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/migrate-iqmod.tcl

vivado-migrate-telemetry:
	SATURN_VIVADO_WRITEBACK=1 $(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/migrate-telemetry.tcl

vivado-build:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/build.tcl

vivado-netlist-audit:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/netlist-audit.tcl

vivado-cdc:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/cdc-audit.tcl

export-prom:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/export-prom.tcl

sim-ddc:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/sim-ddc.tcl

sim-duc:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/sim-duc.tcl

sim-iqmod:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/sim-iqmod.tcl

sim-all:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/test.tcl
