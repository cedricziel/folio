-- W3C trace context (traceparent, tracestate) a job was queued under; NULL for untraced and older jobs

ALTER TABLE jobs ADD COLUMN trace_parent TEXT;
ALTER TABLE jobs ADD COLUMN trace_state TEXT;
