SHELL := /usr/bin/env bash
LAB_DIR := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
VENV_DIR ?= $(HOME)/.venvs/saturn-fpga
VENV_PYTHON := $(VENV_DIR)/bin/python
VIVADO ?= $(LAB_DIR)/scripts/vivado-windows.sh

.PHONY: doctor python-env python-test lint lint-strict formal check vivado-validate vivado-build sim-ddc sim-duc sim-iqmod sim-all

doctor:
	$(LAB_DIR)/scripts/doctor.sh

python-env:
	$(LAB_DIR)/scripts/bootstrap-python.sh

python-test:
	@test -x "$(VENV_PYTHON)" || { echo 'Run make python-env first.' >&2; exit 1; }
	cd $(LAB_DIR) && env -u PYTHONHOME -u PYTHONEXECUTABLE -u PYTHONNOUSERSITE PYTHONPATH=python $(VENV_PYTHON) -m unittest discover -s python/tests -v

lint:
	$(LAB_DIR)/scripts/lint.sh

lint-strict:
	SATURN_LINT_STRICT=1 $(LAB_DIR)/scripts/lint.sh

formal:
	$(LAB_DIR)/scripts/formal.sh

check: lint formal python-test

vivado-validate:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/validate.tcl

vivado-build:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/build.tcl

sim-ddc:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/sim-ddc.tcl

sim-duc:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/sim-duc.tcl

sim-iqmod:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/sim-iqmod.tcl

sim-all:
	$(VIVADO) -mode batch -nolog -nojournal -source $(LAB_DIR)/tcl/test.tcl
