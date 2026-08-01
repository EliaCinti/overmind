-- M16: the language is a property of the company, not of the browser.
--
-- The obvious place would be localStorage — but the server needs it too: the
-- agents' prompts have to say which language to write in, or you would read an
-- Italian interface wrapped around English meeting transcripts and English
-- notifications. One setting, both sides.
--
-- 'en' keeps every existing company exactly as it is.
ALTER TABLE companies ADD COLUMN language TEXT NOT NULL DEFAULT 'en';
