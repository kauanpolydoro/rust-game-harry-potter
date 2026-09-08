.PHONY: check check-reconnect-profile check-lifecycle-profile check-restore-profile dev

check:
	./scripts/check

check-reconnect-profile:
	./scripts/check-reconnect-profile

check-lifecycle-profile:
	./scripts/check-lifecycle-profile

check-restore-profile:
	./scripts/check-restore-profile

dev:
	./scripts/dev
