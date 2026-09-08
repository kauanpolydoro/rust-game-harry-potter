-- Unknown legacy provenance stays NULL; it must not be reported as self-service.
-- This operational attribute is purged with the existing credential graph.
ALTER TABLE recovery_credentials ADD COLUMN host_assisted BOOLEAN;
