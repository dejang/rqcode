import React from "react";

declare const UserAvatar: React.FC<{ children?: React.ReactNode }>;
declare const AvatarFrame: React.FC<{ children?: React.ReactNode }>;
declare const AvatarImage: React.FC<{ source: string }>;
declare const StatusDot: React.FC<{ active: boolean }>;
declare const AvatarCaption: React.FC<{ children?: React.ReactNode }>;
declare const PresenceIndicator: React.FC<{ enabled: boolean }>;

declare const OrderBadge: React.FC<{ children?: React.ReactNode }>;
declare const BadgeShell: React.FC<{ children?: React.ReactNode }>;
declare const BadgeIcon: React.FC<{ glyph: string }>;
declare const PriorityTag: React.FC<{ flagged: boolean }>;
declare const OrderCaption: React.FC<{ children?: React.ReactNode }>;
declare const UrgencyMarker: React.FC<{ critical: boolean }>;

interface AvatarProps {
  src: string;
  name: string;
  online: boolean;
}

interface BadgeProps {
  icon: string;
  code: string;
  urgent: boolean;
}

export function UserAvatarCard({ src, name, online }: AvatarProps) {
  return (
    <UserAvatar>
      <AvatarFrame>
        <AvatarImage source={src} />
        <StatusDot active={online} />
      </AvatarFrame>
      <AvatarCaption>{name}</AvatarCaption>
      <PresenceIndicator enabled={online} />
    </UserAvatar>
  );
}

export function OrderBadgeCard({ icon, code, urgent }: BadgeProps) {
  return (
    <OrderBadge>
      <BadgeShell>
        <BadgeIcon glyph={icon} />
        <PriorityTag flagged={urgent} />
      </BadgeShell>
      <OrderCaption>{code}</OrderCaption>
      <UrgencyMarker critical={urgent} />
    </OrderBadge>
  );
}
