ALTER TABLE task
ADD COLUMN group_certificates_sent boolean;

UPDATE task
SET group_certificates_sent = CASE
    WHEN task_type = 'Group' THEN FALSE
    ELSE NULL
END;
