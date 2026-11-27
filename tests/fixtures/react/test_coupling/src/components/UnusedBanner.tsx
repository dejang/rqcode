import React from "react";

function UnusedBanner({ notice }: { notice: string }) {
  return <div role="status">{notice}</div>;
}

export default React.memo(UnusedBanner);
