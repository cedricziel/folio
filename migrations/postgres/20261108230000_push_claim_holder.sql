-- Who holds a device's lease, so a sender whose lease ran out can neither extend nor end it,
-- and the collapse id an alert keeps across retries.
ALTER TABLE push_devices ADD COLUMN claim_holder UUID;
ALTER TABLE push_devices ADD COLUMN batch_id UUID;
