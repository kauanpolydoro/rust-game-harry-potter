.PHONY: check check-reconnect-profile check-lifecycle-profile dev

check:
	./scripts/check

check-reconnect-profile:
	./scripts/check-reconnect-profile

check-lifecycle-profile:
	./scripts/check-lifecycle-profile

dev:
	./scripts/dev
