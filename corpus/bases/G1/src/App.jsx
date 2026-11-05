import { useEffect, useState } from 'react';
import { calculateStreak, toDateKey, addDays } from './streak.js';

const STORAGE_KEY = 'habit-tracker.habits';

function loadHabits() {
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE_KEY));
    return Array.isArray(saved) ? saved : [];
  } catch {
    return [];
  }
}

function lastDays(today, count) {
  return Array.from({ length: count }, (_, i) => addDays(today, i - count + 1));
}

export default function App() {
  const [habits, setHabits] = useState(loadHabits);
  const [name, setName] = useState('');
  const today = toDateKey(new Date());
  const week = lastDays(today, 7);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(habits));
    } catch {
      // Storage may be unavailable (e.g. private mode); the app still works for this session.
    }
  }, [habits]);

  function addHabit(e) {
    e.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    setHabits([...habits, { id: crypto.randomUUID(), name: trimmed, completed: [] }]);
    setName('');
  }

  function toggle(id, day) {
    setHabits(habits.map((h) => {
      if (h.id !== id) return h;
      const completed = h.completed.includes(day)
        ? h.completed.filter((d) => d !== day)
        : [...h.completed, day];
      return { ...h, completed };
    }));
  }

  function remove(id) {
    const habit = habits.find((h) => h.id === id);
    if (confirm(`Delete "${habit.name}"?`)) {
      setHabits(habits.filter((h) => h.id !== id));
    }
  }

  return (
    <main>
      <h1>Habit Tracker</h1>

      <form onSubmit={addHabit} className="add">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="New habit, e.g. Read 10 pages"
          aria-label="New habit name"
        />
        <button type="submit">Add</button>
      </form>

      {habits.length === 0 ? (
        <p className="empty">No habits yet. Add one above to get started.</p>
      ) : (
        <ul className="habits">
          {habits.map((h) => {
            const streak = calculateStreak(h.completed, today);
            const doneToday = h.completed.includes(today);
            return (
              <li key={h.id} className="habit">
                <div className="row">
                  <button
                    className={`tick ${doneToday ? 'done' : ''}`}
                    onClick={() => toggle(h.id, today)}
                    aria-label={doneToday ? `Untick ${h.name} for today` : `Tick ${h.name} for today`}
                  >
                    {doneToday ? '✓' : ''}
                  </button>
                  <span className="name">{h.name}</span>
                  <span className="streak" title="Current streak">
                    🔥 {streak} {streak === 1 ? 'day' : 'days'}
                  </span>
                  <button className="delete" onClick={() => remove(h.id)} aria-label={`Delete ${h.name}`}>
                    ×
                  </button>
                </div>
                <div className="week">
                  {week.map((day) => {
                    const done = h.completed.includes(day);
                    const label = new Date(`${day}T12:00`).toLocaleDateString(undefined, { weekday: 'short' });
                    return (
                      <button
                        key={day}
                        className={`day ${done ? 'done' : ''} ${day === today ? 'today' : ''}`}
                        onClick={() => toggle(h.id, day)}
                        title={day}
                        aria-label={`${h.name} on ${day}: ${done ? 'done' : 'not done'}`}
                      >
                        {label[0]}
                      </button>
                    );
                  })}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </main>
  );
}
