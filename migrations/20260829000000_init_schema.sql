-- جدول اجرای اصلی (Executions)
CREATE TABLE IF NOT EXISTS executions (
    id TEXT PRIMARY KEY NOT NULL,
    task_type TEXT NOT NULL,
    input_prompt TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL
);

-- جدول تلاش‌ها (Attempts)
CREATE TABLE IF NOT EXISTS attempts (
    id TEXT PRIMARY KEY NOT NULL,
    execution_id TEXT NOT NULL,
    attempt_number INTEGER NOT NULL,
    system_prompt TEXT NOT NULL,
    output TEXT NOT NULL,
    prompt_tokens INTEGER,
    completion_tokens INTEGER,
    latency_ms INTEGER NOT NULL,
    FOREIGN KEY(execution_id) REFERENCES executions(id)
);

-- جدول ارزیابی‌ها (Evaluations)
CREATE TABLE IF NOT EXISTS evaluations (
    id TEXT PRIMARY KEY NOT NULL,
    attempt_id TEXT NOT NULL,
    is_valid BOOLEAN NOT NULL,
    error_category TEXT,
    error_details TEXT,
    FOREIGN KEY(attempt_id) REFERENCES attempts(id)
);

-- جدول درس‌آموخته‌ها (Lessons)
CREATE TABLE IF NOT EXISTS lessons (
    id TEXT PRIMARY KEY NOT NULL,
    task_type TEXT NOT NULL,
    error_category TEXT NOT NULL,
    lesson_learned TEXT NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL
);

-- جدول ثبت سابقه استفاده از درس‌ها (Lesson Usage)
CREATE TABLE IF NOT EXISTS lesson_usage (
    id TEXT PRIMARY KEY NOT NULL,
    lesson_id TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    resulted_in_success BOOLEAN NOT NULL,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP NOT NULL,
    FOREIGN KEY(lesson_id) REFERENCES lessons(id),
    FOREIGN KEY(execution_id) REFERENCES executions(id),
    FOREIGN KEY(attempt_id) REFERENCES attempts(id)
);

-- ایندکس‌های FTS5 و ترکیبی برای سرعت کوئری‌گیری
CREATE INDEX IF NOT EXISTS idx_lessons_task_error ON lessons(task_type, error_category);
