import React from "react";

interface InputProps {
  label: string;
}

export const ForwardedInput = React.forwardRef<HTMLInputElement, InputProps>(
  ({ label }, ref) => {
    return (
      <label>
        <span>{label}</span>
        <input ref={ref} type="text" />
      </label>
    );
  },
);
