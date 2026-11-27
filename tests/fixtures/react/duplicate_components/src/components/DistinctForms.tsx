import React from "react";

interface FormValues {
  first: string;
  second: string;
  locked: boolean;
}

export function ProfileForm({ first, second, locked }: FormValues) {
  return (
    <form method="post" autoComplete="on" noValidate={locked}>
      <input
        name="fullName"
        placeholder="Full name"
        aria-label="Full name"
        autoComplete="name"
        maxLength={80}
        defaultValue={first}
      />
      <input
        name="emailAddress"
        placeholder="Email"
        aria-label="Email"
        autoComplete="email"
        maxLength={120}
        defaultValue={second}
      />
      <button type="submit" disabled={locked}>
        Save
      </button>
    </form>
  );
}

export function FilterForm({ first, second, locked }: FormValues) {
  return (
    <form role="search" target="_self" hidden={locked}>
      <input
        id="query-input"
        title="Search term"
        inputMode="search"
        list="recent-queries"
        minLength={2}
        value={first}
      />
      <input
        id="category-input"
        title="Category"
        inputMode="text"
        list="category-options"
        minLength={1}
        value={second}
      />
      <button type="reset" aria-disabled={locked}>
        Clear
      </button>
    </form>
  );
}
