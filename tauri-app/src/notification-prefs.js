export function notificationPrefsPayload({ enabled, taskCompleted, interaction }) {
  return {
    enabled,
    taskCompleted,
    interaction,
  };
}
