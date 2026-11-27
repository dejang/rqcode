import React from "react";
import { observer } from "mobx-react-lite";

export default observer(ObservedCard);

function ObservedCard({ title }: { title: string }) {
  return <article>{title}</article>;
}
