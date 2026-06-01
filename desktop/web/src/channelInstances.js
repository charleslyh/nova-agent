export const CHANNEL_TYPES = [
  { id: "qq", label: "QQ 机器人" },
  { id: "wecom", label: "企业微信" }
];

const PLATFORM_NAMES = Object.fromEntries(CHANNEL_TYPES.map((t) => [t.id, t.label]));

/** Map `GET /channels` row for sidebar lookup. */
export function mapChannelInstance(item) {
  if (!item?.type || !item?.session_id || !item?.channel_id) {
    return null;
  }
  return {
    type: item.type,
    channelId: item.channel_id,
    sessionId: item.session_id,
    platformName: PLATFORM_NAMES[item.type] ?? item.type
  };
}

export function channelTypeBySessionIdFromInstances(instances) {
  const map = new Map();
  for (const inst of instances ?? []) {
    if (inst?.sessionId && inst?.type) {
      map.set(inst.sessionId, inst.type);
    }
  }
  return map;
}
