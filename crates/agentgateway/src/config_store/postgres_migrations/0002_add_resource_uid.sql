ALTER TABLE agw_config_resources
	ADD COLUMN uid UUID NOT NULL UNIQUE DEFAULT gen_random_uuid();
