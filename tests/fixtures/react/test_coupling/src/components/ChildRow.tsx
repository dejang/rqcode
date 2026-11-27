import React from "react";
import { Link } from "react-router-dom";
import styles from "./ChildRow.module.css";

export function ChildRow({ label }: { label: string }) {
  return (
    <li className={styles.row}>
      <Link to={`/items/${label}`}>{label}</Link>
    </li>
  );
}
