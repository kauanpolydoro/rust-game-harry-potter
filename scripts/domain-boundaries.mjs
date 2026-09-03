export function findUnapprovedDependencies(dependencies, allowedDependencies) {
  const allowed = new Set(allowedDependencies)

  return dependencies
    .map((dependency) => dependency.name)
    .filter((dependency) => !allowed.has(dependency))
    .sort()
}
