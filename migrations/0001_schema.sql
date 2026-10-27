-- learn.xabcnews.com: courses XABC writes and keeps current, a lesson at a time.

-- A course: its address, the group it stands in, its name, a line on what it teaches, and the
-- brief XABC writes it to (readers, scope, stack, the route of its chapters, the documents it
-- follows). Courses stand in the order they came (`place`); a hidden one is neither listed nor
-- extended. `idea` keeps what opened it and every sentence that turned it, `direction` and
-- `adjusted_at` the last turn. `assets` is the address of the site's own files.
CREATE TABLE courses (
    id text PRIMARY KEY CHECK (id ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND char_length(id) <= 40 AND id <> 'assets'),
    place integer GENERATED ALWAYS AS IDENTITY,
    grp text NOT NULL CHECK (char_length(grp) BETWEEN 1 AND 20),
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 40),
    intro text NOT NULL CHECK (char_length(intro) BETWEEN 1 AND 120),
    brief text NOT NULL CHECK (char_length(brief) BETWEEN 1 AND 4000),
    idea text NOT NULL DEFAULT '',
    source text NOT NULL CHECK (source IN ('seed', 'request', 'discovery')),
    visible boolean NOT NULL DEFAULT true,
    direction text NOT NULL DEFAULT '',
    adjusted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- A lesson: its place in its course (`number`) under the chapter it names, its title and Markdown
-- body, the start of its words for lists, its cover among its pictures, how many times it has been
-- written (`revision`) and when it is reviewed next.
CREATE TABLE lessons (
    id uuid PRIMARY KEY,
    course text NOT NULL REFERENCES courses (id),
    number integer NOT NULL CHECK (number > 0),
    chapter text NOT NULL CHECK (char_length(chapter) BETWEEN 1 AND 40),
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 100),
    body text NOT NULL,
    excerpt text NOT NULL,
    cover uuid,
    revision integer NOT NULL DEFAULT 1 CHECK (revision > 0),
    review_at timestamptz NOT NULL,
    published_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (course, number)
);
CREATE INDEX lessons_review_idx ON lessons (review_at);

-- The pictures a lesson shows, kept on tmpfile; its body names one as `image:<id>`.
CREATE TABLE images (
    id uuid PRIMARY KEY,
    lesson uuid NOT NULL REFERENCES lessons (id) ON DELETE CASCADE,
    url text NOT NULL,
    content_type text NOT NULL,
    width integer NOT NULL CHECK (width > 0),
    height integer NOT NULL CHECK (height > 0),
    expires_at timestamptz NOT NULL
);
CREATE INDEX images_lesson_idx ON images (lesson);
ALTER TABLE lessons ADD CONSTRAINT lessons_cover_fkey FOREIGN KEY (cover) REFERENCES images (id);

-- The day a task that runs now and then last ran: `discovery`, the weekly look for a new course.
CREATE TABLE tasks (
    name text PRIMARY KEY,
    done_on date NOT NULL
);
