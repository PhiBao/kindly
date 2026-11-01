-- How a client message was read: the confidence and the compact judgment that
-- produced the reading. Kept beside the message so the owner can always see who
-- read it and how sure they were.

ALTER TABLE messages ADD COLUMN confidence REAL;
ALTER TABLE messages ADD COLUMN judgment_json TEXT;
