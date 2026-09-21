export function notificationPrefsPayload({ enabled, taskCompleted, interaction, error, detail }) {
  return {
    enabled,
    taskCompleted,
    interaction,
    error,
    detail,
  };
}
